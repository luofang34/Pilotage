//! The single task owning [`SessionEngine`] and the embedded reference
//! adapter (ADR-0002, ADR-0005): every connection task funnels decoded
//! messages here instead of touching engine state directly, so the state
//! machine is driven from exactly one place.

use std::time::Duration;

use pilotage_adapter_api::{StepBudget, VehicleAdapter};
use pilotage_session::{ClientKey, DomainEnvelope, SessionAction, SessionEngine, SessionOutcome};
use pilotage_timing::{BoundedLatencyLog, MonoTimestamp, Stage, StageLatency};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use crate::runtime::connection::{DatagramClass, ToConnection};
use crate::runtime::registry::ClientRegistry;
use crate::runtime::wire_codec::{
    encode_envelope_message, encode_pong_datagram, encode_telemetry_datagram,
};

mod telemetry;
use telemetry::sample_to_wire;

#[cfg(test)]
mod tests;

mod link_loss;

/// Capacity of the [`EngineActor`]'s inbound command queue.
///
/// Bounded per ADR-0009's discipline: a lagging engine actor is a
/// correctness signal (dropped commands are counted below), never silent
/// backpressure.
pub const ENGINE_QUEUE_CAPACITY: usize = 1024;

/// How often the embedded adapter is stepped and telemetry broadcast
/// (ADR-0005, ADR-0009's control/telemetry cadence).
pub const TICK_INTERVAL: Duration = Duration::from_millis(10);

/// Capacity of the per-stage latency ring buffer dumped at shutdown.
const LATENCY_LOG_CAPACITY: usize = 4096;

/// The ADR-0011 message classes the engine actor fans messages out as, for
/// per-class drop accounting (ADR-0009: "drops are counted, never silent";
/// ADR-0011: "drops are counted per class").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageClass {
    /// A unicast reply to one client (bootstrap/session state or `Pong`).
    Unicast,
    /// A reliable, ordered authority/mode-change broadcast.
    AuthorityBroadcast,
    /// Best-effort telemetry, fanned out at tick cadence.
    Telemetry,
}

/// Wrapping per-class counters for messages dropped because a client's
/// outbound queue could not accept them without blocking the actor task.
#[derive(Debug, Default)]
struct DropCounters {
    unicast: u64,
    authority_broadcast: u64,
    telemetry: u64,
}

impl DropCounters {
    fn record(&mut self, class: MessageClass) -> u64 {
        let counter = match class {
            MessageClass::Unicast => &mut self.unicast,
            MessageClass::AuthorityBroadcast => &mut self.authority_broadcast,
            MessageClass::Telemetry => &mut self.telemetry,
        };
        *counter = counter.wrapping_add(1);
        *counter
    }
}

/// One command a connection task submits to the engine actor.
#[derive(Debug)]
pub enum ToEngine {
    /// A client connected; the actor should register its outbound sender in
    /// the client registry keyed on `client`.
    ClientConnected {
        /// Driver-assigned key for the new connection.
        client: ClientKey,
        /// Outbound sender the actor unicasts/broadcasts through.
        sender: mpsc::Sender<ToConnection>,
    },
    /// A decoded client message ready for [`SessionEngine::handle_client_message`].
    ClientMessage {
        /// Sender of the message.
        client: ClientKey,
        /// The decoded message.
        message: DomainEnvelope,
        /// The driver's receive timestamp, for staleness/latency accounting.
        now: MonoTimestamp,
    },
    /// A one-shot latency summary request, used by the shutdown path to log
    /// the accumulated per-stage timings before the process exits.
    DumpLatencySummary {
        /// Where to send the formatted summary line.
        reply: oneshot::Sender<String>,
    },
}

/// Owns the [`SessionEngine`], the embedded vehicle adapter, the client
/// registry, and the per-stage latency log; runs until its command channel
/// closes.
///
/// Generic over the [`VehicleAdapter`] so the same actor drives either the
/// deterministic reference adapter or the real Gazebo adapter for control,
/// telemetry, and stepping. Video frames are not part of this trait and are
/// wired separately (the media task, ADR-0005/0008).
pub struct EngineActor<A: VehicleAdapter> {
    engine: SessionEngine,
    adapter: A,
    clients: ClientRegistry,
    latency: BoundedLatencyLog<LATENCY_LOG_CAPACITY>,
    drops: DropCounters,
    /// Wrapping count of link-loss policy changes the adapter failed to
    /// enact. A non-zero value is a fail-closed fault, not noise: authority
    /// was already fenced, so an unenacted policy means the vehicle may
    /// still be executing its last command with nobody in control.
    link_loss_enact_failures: u64,
    /// The single monotonic origin shared with every connection task's
    /// client-message stamps (ADR-0009: one `host_time` reference domain).
    /// Passed in rather than sampled here so tick-driven timestamps and
    /// client-message timestamps are never comparing two different clocks.
    start: Instant,
}

impl<A: VehicleAdapter> EngineActor<A> {
    /// Constructs an actor wrapping `engine` and `adapter`, with an empty
    /// client registry and latency log.
    ///
    /// `start` must be the same monotonic origin used to stamp incoming
    /// client messages (ADR-0009): the actor derives every tick and
    /// offer-expiry timestamp from it, and a second, independently-sampled
    /// origin would skew those comparisons against client-message stamps.
    #[must_use]
    pub fn new(engine: SessionEngine, adapter: A, start: Instant) -> Self {
        Self {
            engine,
            adapter,
            clients: ClientRegistry::new(),
            latency: BoundedLatencyLog::new(),
            drops: DropCounters::default(),
            link_loss_enact_failures: 0,
            start,
        }
    }

    /// Runs the actor loop: services `commands` and a fixed-cadence tick
    /// interval until the command channel closes.
    pub async fn run(mut self, mut commands: mpsc::Receiver<ToEngine>) {
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                command = commands.recv() => {
                    match command {
                        Some(command) => self.handle_command(command),
                        None => break,
                    }
                }
                _ = ticker.tick() => {
                    self.on_tick();
                }
            }
        }
        info!("engine actor command channel closed, shutting down");
    }

    fn handle_command(&mut self, command: ToEngine) {
        match command {
            ToEngine::ClientConnected { client, sender } => {
                self.clients.insert(client, sender);
            }
            ToEngine::ClientMessage {
                client,
                message,
                now,
            } => {
                self.on_client_message(client, message, now);
            }
            ToEngine::DumpLatencySummary { reply } => {
                let summary = self.latency_summary();
                // The receiver may have already given up waiting; a dropped
                // reply is not a correctness issue for the engine itself.
                reply.send(summary).ok();
            }
        }
    }

    fn on_client_message(
        &mut self,
        client: ClientKey,
        message: DomainEnvelope,
        now: MonoTimestamp,
    ) {
        let validate_start = Instant::now();
        let is_disconnect = matches!(message, DomainEnvelope::Disconnect);
        let outcome = self.engine.handle_client_message(client, message, now);
        self.record_stage(Stage::Validate, validate_start.elapsed());
        if is_disconnect {
            self.clients.remove(client);
        }
        self.enact(outcome);
    }

    fn on_tick(&mut self) {
        let now = MonoTimestamp::from_nanos(u64_nanos_since(self.start));
        let outcome = self.engine.handle_tick(now);
        self.enact(outcome);

        let apply_start = Instant::now();
        let step_outcome = self.adapter.step(StepBudget { ticks: 1 });
        self.record_stage(Stage::Apply, apply_start.elapsed());
        debug!(advanced = step_outcome.advanced, "adapter stepped");

        self.broadcast_telemetry(now);
    }

    fn broadcast_telemetry(&mut self, now: MonoTimestamp) {
        let batch = self.adapter.sample_telemetry();
        for sample in batch.samples {
            let wire_sample = sample_to_wire(sample, now);
            let bytes = encode_telemetry_datagram(wire_sample);
            self.broadcast_datagram(bytes);
        }
    }

    fn enact(&mut self, outcome: SessionOutcome) {
        if outcome.dropped > 0 {
            error!(
                dropped = outcome.dropped,
                "session engine dropped actions at the per-call cap"
            );
        }
        for action in outcome.actions {
            self.enact_one(action);
        }
    }

    fn enact_one(&mut self, action: SessionAction) {
        match action {
            SessionAction::SendToClient { client, envelope } => {
                // `Pong` is the ADR-0009 RTT probe reply and travels as a
                // datagram alongside `Ping` (ADR-0005's channel mapping);
                // `Authority` (were it ever unicast) belongs on the
                // dedicated authority-events stream, never sharing the
                // bootstrap stream's head-of-line with bulk/handshake
                // traffic; every other unicast reply is session
                // bootstrap/lease state and belongs on the bootstrap stream.
                let message = to_connection_message(&envelope);
                self.send_to(client, message, MessageClass::Unicast);
            }
            SessionAction::Broadcast { envelope } => {
                let message = to_connection_message(&envelope);
                self.broadcast(message, MessageClass::AuthorityBroadcast);
            }
            SessionAction::RejectFrame { client, rejection } => {
                let bytes = crate::runtime::wire_codec::encode_frame_rejected_datagram(&rejection);
                self.send_to(
                    client,
                    ToConnection::Datagram {
                        class: DatagramClass::FrameRejected,
                        bytes,
                    },
                    MessageClass::Unicast,
                );
            }
            SessionAction::CloseClient { client, reason } => {
                warn!(
                    ?reason,
                    client = client.as_u64(),
                    "closing client connection"
                );
                self.send_to(client, ToConnection::Close, MessageClass::Unicast);
                self.clients.remove(client);
            }
            SessionAction::ApplyToAdapter { frame } => {
                let apply_start = Instant::now();
                let outcome = self.adapter.apply_control(&frame);
                self.record_stage(Stage::Apply, apply_start.elapsed());
                debug!(?outcome, "control frame applied to adapter");
            }
            action @ (SessionAction::EngageLinkLoss { .. }
            | SessionAction::ClearLinkLoss { .. }) => {
                self.enact_link_loss(action);
            }
        }
    }

    /// Hands `message` to `client`'s connection task without ever blocking
    /// this actor's single task on that client's outbound queue (ADR-0009: a
    /// stalled client must not wedge ticks, telemetry, or every other
    /// client's control processing).
    ///
    /// A full queue means the client is draining slower than it is being fed
    /// and is a counted, non-fatal drop; only a closed channel — the
    /// connection task has actually exited — evicts the client from the
    /// registry.
    fn send_to(&mut self, client: ClientKey, message: ToConnection, class: MessageClass) {
        let Some(sender) = self.clients.sender(client) else {
            return;
        };
        let reliable = targets_reliable_stream(&message);
        match sender.try_send(message) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                let total = self.drops.record(class);
                if reliable {
                    // Silently dropping an ordered reliable event breaks the
                    // stream's ordering guarantee for every later event; a
                    // client that cannot keep up with its own bootstrap/
                    // authority stream is closed instead (ADR-0011).
                    error!(
                        client = client.as_u64(),
                        ?class,
                        total_dropped = total,
                        "reliable-stream outbound queue full, closing connection"
                    );
                    self.close_full_client(client);
                } else {
                    warn!(
                        client = client.as_u64(),
                        ?class,
                        total_dropped = total,
                        "client outbound queue full, dropping message"
                    );
                }
            }
            Err(TrySendError::Closed(_)) => {
                self.clients.remove(client);
            }
        }
    }

    /// Closes a client whose outbound queue overflowed on a reliable-stream
    /// message: sends a best-effort `Close` (which the writer honors before it
    /// drains the backlog) and evicts the client so no further messages are
    /// routed to it.
    fn close_full_client(&mut self, client: ClientKey) {
        if let Some(sender) = self.clients.sender(client) {
            sender.try_send(ToConnection::Close).ok();
        }
        self.clients.remove(client);
    }

    fn broadcast_datagram(&mut self, bytes: Vec<u8>) {
        self.broadcast(
            ToConnection::Datagram {
                class: DatagramClass::Telemetry,
                bytes,
            },
            MessageClass::Telemetry,
        );
    }

    /// Fans `message` out to every connected client without blocking the
    /// actor task on any single client's queue (ADR-0009), evicting a client
    /// only when its connection task has actually gone away — never merely
    /// because it briefly could not keep up (ADR-0011: telemetry and
    /// authority broadcasts have different reliability needs but neither
    /// should disconnect a client over one busy tick).
    fn broadcast(&mut self, message: ToConnection, class: MessageClass) {
        let reliable = targets_reliable_stream(&message);
        let mut closed = Vec::new();
        let mut full = Vec::new();
        for (client, sender) in self.clients.iter() {
            match sender.try_send(message.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => full.push(client),
                Err(TrySendError::Closed(_)) => closed.push(client),
            }
        }
        if !full.is_empty() {
            let total = (0..full.len())
                .map(|_| self.drops.record(class))
                .last()
                .unwrap_or_default();
            // A best-effort telemetry broadcast tolerates a full queue; a
            // reliable authority broadcast does not — dropping it would break
            // ordering for that client, so it is closed instead (ADR-0011).
            if reliable {
                error!(
                    ?class,
                    skipped_clients = full.len(),
                    total_dropped = total,
                    "reliable-stream broadcast queue full, closing affected connections"
                );
            } else {
                debug!(
                    ?class,
                    skipped_clients = full.len(),
                    total_dropped = total,
                    "broadcast skipped clients with a full outbound queue"
                );
            }
        }
        if reliable {
            for client in full {
                self.close_full_client(client);
            }
        }
        for client in closed {
            self.clients.remove(client);
        }
    }

    fn record_stage(&mut self, stage: Stage, duration: Duration) {
        if !self.latency.push(StageLatency::new(stage, duration)) {
            debug!(
                ?stage,
                dropped = self.latency.dropped(),
                "latency log evicted an entry"
            );
        }
    }

    /// Builds the shutdown-time latency summary line dumped via `tracing`.
    fn latency_summary(&self) -> String {
        let mean = self.latency.mean();
        let max = self.latency.max();
        format!(
            "stages_recorded={} dropped={} mean={:?} max={:?}",
            self.latency.len(),
            self.latency.dropped(),
            mean,
            max.map(|record| record.duration)
        )
    }
}

/// Nanoseconds elapsed since `start`, saturating rather than overflowing for
/// an implausibly long-running process.
fn u64_nanos_since(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Whether `message` is destined for one of the reliable ordered streams
/// (bootstrap or authority-events, ADR-0005), where a dropped message would
/// break the stream's ordering guarantee. Best-effort datagrams and the
/// `Close` signal are not.
fn targets_reliable_stream(message: &ToConnection) -> bool {
    matches!(
        message,
        ToConnection::BootstrapMessage(_) | ToConnection::AuthorityMessage(_)
    )
}

/// Picks the [`ToConnection`] channel arm an [`OutboundMessage`] belongs on
/// and encodes it (ADR-0005): `Pong` is a control-fast datagram; `Authority`
/// travels on the dedicated authority-events stream so a bulk/bootstrap
/// transfer can never head-of-line-block a lease/override event; every other
/// arm is bootstrap/lease/handshake reply traffic on the bootstrap stream.
fn to_connection_message(envelope: &pilotage_session::OutboundMessage) -> ToConnection {
    match envelope {
        pilotage_session::OutboundMessage::Pong(pong) => ToConnection::Datagram {
            class: DatagramClass::Pong,
            bytes: encode_pong_datagram(pong),
        },
        pilotage_session::OutboundMessage::Authority(_) => {
            ToConnection::AuthorityMessage(encode_envelope_message(envelope))
        }
        pilotage_session::OutboundMessage::Welcome(_)
        | pilotage_session::OutboundMessage::LeaseResponse(_)
        | pilotage_session::OutboundMessage::LeaseReleased(_) => {
            ToConnection::BootstrapMessage(encode_envelope_message(envelope))
        }
    }
}

//! The single task owning the engine (ADR-0002, ADR-0005): every
//! connection task funnels decoded messages here instead of touching
//! engine state directly, so the state machine is driven from exactly one
//! place. The engine itself, with the session engine and the adapter, is
//! the host core's ([`pilotage_host_core::engine_actor::EngineActor`]);
//! this task gives it the time, delivers each client's queued messages to
//! that client's connection task, and adds what only a process host has:
//! the mission lane, the flight recorder, the simulator's truth lane and
//! the latency log.

use std::time::Duration;

use pilotage_adapter_api::VehicleAdapter;
#[cfg(feature = "sim")]
use pilotage_host_core::engine_actor::TelemetryTap;
use pilotage_host_core::engine_actor::{
    EngineActor as CoreEngine, MessageClass, ToEngine as CoreCommand,
};
use pilotage_session::{ClientKey, SessionEngine};
use pilotage_timing::{BoundedLatencyLog, MonoTimestamp, Stage, StageLatency};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::time::Instant;
use tracing::{debug, error, info};

use crate::runtime::connection::ToConnection;
use crate::runtime::registry::ClientRegistry;

mod command;
mod drops;
mod mission;
#[cfg(feature = "sim")]
mod recorder;
#[cfg(feature = "sim")]
mod sim_truth_lane;
pub use command::ToEngine;
use drops::DropCounters;

/// Capacity of the [`EngineActor`]'s inbound command queue.
///
/// Bounded per ADR-0009's discipline: a lagging engine actor is a
/// correctness signal (dropped commands are counted below), never silent
/// backpressure.
pub const ENGINE_QUEUE_CAPACITY: usize = 1024;

/// How often the embedded adapter is stepped and telemetry broadcast
/// (ADR-0005, ADR-0009's control/telemetry cadence).
pub const TICK_INTERVAL: Duration = pilotage_host_core::engine_actor::TICK_INTERVAL;

/// Capacity of the per-stage latency ring buffer dumped at shutdown.
const LATENCY_LOG_CAPACITY: usize = 4096;

/// Runs the host core's engine for the process host's connections.
///
/// Generic over the [`VehicleAdapter`] so the same actor drives every
/// adapter the host supports. Video frames are not part of this trait and
/// are wired separately (the media task, ADR-0005/0008).
pub struct EngineActor<A: VehicleAdapter> {
    core: CoreEngine<A>,
    clients: ClientRegistry,
    latency: BoundedLatencyLog<LATENCY_LOG_CAPACITY>,
    drops: DropCounters,
    /// The mission lane that receives client intents (ADR-0052).
    mission_lane: Option<mpsc::Sender<crate::runtime::mission_lane::LaneInbound>>,
    /// The simulation flight recorder, when `PILOTAGE_RECORD_DIR` asks
    /// for one. Simulation-only by construction (ADR-0040).
    #[cfg(feature = "sim")]
    recorder: Option<recorder::Recorder>,
    /// The simulator's own truth, when `PILOTAGE_SIM_TRUTH_ADDR` names
    /// where it arrives; simulation-only like the recorder.
    #[cfg(feature = "sim")]
    truth_lane: Option<sim_truth_lane::SimTruthLane>,
    /// The single monotonic origin shared with every connection task's
    /// client-message stamps (ADR-0009: one `host_time` reference domain).
    start: Instant,
}

impl<A: VehicleAdapter> EngineActor<A> {
    /// Constructs an actor wrapping `engine` and `adapter`, with an empty
    /// client registry and latency log.
    ///
    /// `start` must be the same monotonic origin used to stamp incoming
    /// client messages (ADR-0009). The magnetic field is evaluated at the
    /// host's civil time when the actor starts.
    #[must_use]
    pub fn new(engine: SessionEngine, adapter: A, start: Instant) -> Self {
        Self {
            core: CoreEngine::new(engine, adapter, host_unix_seconds()),
            clients: ClientRegistry::new(),
            latency: BoundedLatencyLog::new(),
            drops: DropCounters::default(),
            mission_lane: None,
            #[cfg(feature = "sim")]
            recorder: recorder::Recorder::from_env(),
            #[cfg(feature = "sim")]
            truth_lane: sim_truth_lane::SimTruthLane::from_env(),
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
        self.core.set_now(self.start.elapsed());
        match command {
            ToEngine::ClientConnected { client, sender } => {
                self.clients.insert(client, sender);
                self.core
                    .handle_command(CoreCommand::ClientConnected { client });
            }
            ToEngine::ClientMessage {
                client,
                message,
                now,
            } => {
                let is_disconnect =
                    matches!(*message, pilotage_session::DomainEnvelope::Disconnect);
                let validate_start = Instant::now();
                self.core.handle_command(CoreCommand::ClientMessage {
                    client,
                    message,
                    now,
                });
                self.record_stage(Stage::Validate, validate_start.elapsed());
                self.deliver();
                if is_disconnect {
                    self.clients.remove(client);
                }
                return;
            }
            ToEngine::NavGuidance { vehicle, state } => {
                self.core
                    .handle_command(CoreCommand::NavGuidance { vehicle, state });
            }
            ToEngine::MissionIntent { client, intent } => self.on_mission_intent(client, intent),
            ToEngine::Requester { client, requester } => self.on_requester(client, requester),
            ToEngine::MissionPublish { to, payload } => self.publish_mission(to, *payload),
            ToEngine::MissionLane { sender } => self.mission_lane = Some(sender),
            ToEngine::DumpLatencySummary { reply } => {
                let summary = self.latency_summary();
                // The receiver may have already given up waiting; a dropped
                // reply is not a correctness issue for the engine itself.
                reply.send(summary).ok();
            }
        }
        self.deliver();
    }

    fn on_tick(&mut self) {
        self.core.set_now(self.start.elapsed());
        let apply_start = Instant::now();
        #[cfg(feature = "sim")]
        let mut tap = SimTap {
            recorder: self.recorder.as_mut(),
            truth: self
                .truth_lane
                .as_mut()
                .and_then(sim_truth_lane::SimTruthLane::poll),
        };
        #[cfg(not(feature = "sim"))]
        let mut tap = pilotage_host_core::engine_actor::NoTap;
        self.core.tick_with(&mut tap);
        self.record_stage(Stage::Apply, apply_start.elapsed());
        self.deliver();
    }

    /// Hands every message the engine queued to its client's connection
    /// task without ever blocking this actor's single task on that
    /// client's outbound queue (ADR-0009: a stalled client must not wedge
    /// ticks, telemetry, or every other client's control processing).
    ///
    /// A full queue is a counted drop; a full reliable stream closes the
    /// client instead, since a dropped ordered event breaks the stream's
    /// ordering for every later event (ADR-0011). Only a closed channel —
    /// the connection task has actually exited — evicts the client without
    /// a close.
    fn deliver(&mut self) {
        for client in self.core.client_keys() {
            for message in self.core.take_messages(client) {
                if !self.send_to(client, message) {
                    break;
                }
            }
        }
    }

    /// Sends one message to one client; false when the client is gone.
    fn send_to(&mut self, client: ClientKey, message: ToConnection) -> bool {
        let Some(sender) = self.clients.sender(client) else {
            return false;
        };
        let reliable = targets_reliable_stream(&message);
        let class = class_of(&message);
        match sender.try_send(message) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                let total = self.drops.record(class);
                if reliable {
                    error!(
                        client = client.as_u64(),
                        ?class,
                        total_dropped = total,
                        "reliable-stream outbound queue full, closing connection"
                    );
                    self.close_full_client(client);
                    false
                } else {
                    debug!(
                        client = client.as_u64(),
                        ?class,
                        total_dropped = total,
                        "client outbound queue full, dropping message"
                    );
                    true
                }
            }
            Err(TrySendError::Closed(_)) => {
                self.forget(client);
                false
            }
        }
    }

    /// Closes a client whose outbound queue overflowed on a reliable-stream
    /// message: sends a best-effort `Close` (which the writer honors before
    /// it drains the backlog) and evicts the client.
    fn close_full_client(&mut self, client: ClientKey) {
        if let Some(sender) = self.clients.sender(client) {
            sender.try_send(ToConnection::Close).ok();
        }
        self.forget(client);
    }

    fn forget(&mut self, client: ClientKey) {
        self.clients.remove(client);
        self.core.disconnect(client);
    }

    /// Fans `message` out to every connected client, through the engine's
    /// queues so it keeps its order behind the engine's own messages.
    fn broadcast(&mut self, message: ToConnection, class: MessageClass) {
        self.core.broadcast_message(message, class);
        self.deliver();
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

/// Whether `message` is destined for one of the reliable ordered streams
/// (bootstrap or authority-events, ADR-0005), where a dropped message would
/// break the stream's ordering guarantee. Best-effort datagrams and the
/// `Close` signal are not.
fn targets_reliable_stream(message: &ToConnection) -> bool {
    matches!(
        message,
        ToConnection::BootstrapMessage { .. } | ToConnection::AuthorityMessage(_)
    )
}

/// The ADR-0011 class a message's drop counts against.
fn class_of(message: &ToConnection) -> MessageClass {
    match message {
        ToConnection::Datagram {
            class: pilotage_host_core::connection::DatagramClass::Telemetry,
            ..
        } => MessageClass::Telemetry,
        ToConnection::AuthorityMessage(_) => MessageClass::AuthorityBroadcast,
        _ => MessageClass::Unicast,
    }
}

/// The host's civil time, Unix seconds: the date the magnetic field is
/// evaluated at for the whole run.
fn host_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The simulation build's telemetry tap: the simulator's truth fills a
/// sample the adapter left without one, and the recorder keeps every
/// corrected sample.
#[cfg(feature = "sim")]
struct SimTap<'a> {
    recorder: Option<&'a mut recorder::Recorder>,
    truth: Option<pilotage_adapter_api::SimTruthSample>,
}

#[cfg(feature = "sim")]
impl TelemetryTap for SimTap<'_> {
    fn fill(&mut self, sample: &mut pilotage_adapter_api::TelemetrySample) {
        // The adapter's own truth takes precedence over the side lane.
        if sample.sim_truth.is_none() {
            sample.sim_truth = self.truth;
        }
    }

    fn record(&mut self, sample: &pilotage_adapter_api::TelemetrySample, now: MonoTimestamp) {
        if let Some(recorder) = self.recorder.as_mut() {
            recorder.record(sample, now);
        }
    }
}

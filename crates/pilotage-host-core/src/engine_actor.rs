//! The session engine and the vehicle adapter, driven from one place:
//! the session host's engine actor without its task, its channels or its
//! clock. Every in-core client funnels its messages here, the engine's
//! replies queue for each client, and the core passes the time on each
//! call (ADR-0002, ADR-0005).

use std::collections::BTreeMap;
use std::time::Duration;

use pilotage_adapter_api::{StepBudget, VehicleAdapter};
use pilotage_protocol::{VehicleId, wire};
use pilotage_session::{ClientKey, DomainEnvelope, SessionAction, SessionEngine, SessionOutcome};
use pilotage_timing::MonoTimestamp;
use tracing::{debug, error, info, warn};

use crate::connection::{DatagramClass, ToConnection};
use crate::registry::ClientRegistry;

mod command;
mod outbound;
mod telemetry;
pub use command::ToEngine;
pub use drops::MessageClass;
pub use outbound::to_connection_message;

mod action_dedup;
mod apply;
mod drops;
mod link_loss;
mod rejection_dedup;
use action_dedup::ActionDedup;
use pilotage_protocol::ScopedControlFrame;
use rejection_dedup::RejectionDedup;

/// How often the embedded adapter is stepped and telemetry broadcast
/// (ADR-0005, ADR-0009's control/telemetry cadence).
pub const TICK_INTERVAL: Duration = Duration::from_millis(10);

/// What a host adds to each telemetry sample the engine broadcasts: a
/// simulator's truth before the corrections, a recorder after them.
pub trait TelemetryTap {
    /// Fills the sample before the host's corrections.
    fn fill(&mut self, sample: &mut pilotage_adapter_api::TelemetrySample);
    /// Sees the corrected sample at `now`.
    fn record(&mut self, sample: &pilotage_adapter_api::TelemetrySample, now: MonoTimestamp);
}

/// The tap of a host that adds nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTap;

impl TelemetryTap for NoTap {
    fn fill(&mut self, _sample: &mut pilotage_adapter_api::TelemetrySample) {}
    fn record(&mut self, _sample: &pilotage_adapter_api::TelemetrySample, _now: MonoTimestamp) {}
}

/// Owns the [`SessionEngine`], the vehicle adapter and the client queues.
///
/// Generic over the [`VehicleAdapter`], as the session host's actor is.
pub struct EngineActor<A: VehicleAdapter> {
    engine: SessionEngine,
    adapter: A,
    clients: ClientRegistry,
    action_dedup: ActionDedup,
    adapter_rejection_dedup: RejectionDedup,
    /// Wrapping count of link-loss policy changes the adapter failed to
    /// enact. A non-zero value is a fail-closed fault, not noise: authority
    /// was already fenced, so an unenacted policy means the vehicle may
    /// still be executing its last command with nobody in control.
    link_loss_enact_failures: u64,
    /// Latest navigation guidance per vehicle (ADR-0031). A vehicle with
    /// no entry has no guidance to show: its telemetry carries the field
    /// absent rather than a centered or zeroed one.
    nav_guidance: BTreeMap<VehicleId, wire::NavGuidanceState>,
    /// Turns a declared magnetic heading true with the shared model.
    magnetic: telemetry::MagneticCorrection,
    /// The time of the current call, since the core's origin: every tick,
    /// offer-expiry and admission-budget time comes from it (ADR-0009:
    /// one `host_time` reference domain).
    now: Duration,
}

impl<A: VehicleAdapter> EngineActor<A> {
    /// An actor wrapping `engine` and `adapter`, with no clients, at time
    /// zero. The magnetic field is evaluated at `magnetic_epoch_unix_s`.
    #[must_use]
    pub fn new(engine: SessionEngine, adapter: A, magnetic_epoch_unix_s: i64) -> Self {
        Self {
            engine,
            adapter,
            clients: ClientRegistry::new(),
            action_dedup: ActionDedup::default(),
            adapter_rejection_dedup: RejectionDedup::default(),
            link_loss_enact_failures: 0,
            nav_guidance: BTreeMap::new(),
            magnetic: telemetry::MagneticCorrection::at_unix_seconds(magnetic_epoch_unix_s),
            now: Duration::ZERO,
        }
    }

    /// Sets the time of the current call.
    pub fn set_now(&mut self, now: Duration) {
        self.now = now;
    }

    /// The adapter, for the bytes and time of its links.
    pub fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }

    /// Takes the messages queued for `client`, in order.
    pub fn take_messages(&mut self, client: ClientKey) -> std::collections::VecDeque<ToConnection> {
        self.clients.take(client)
    }

    /// Steps the adapter and the engine once and broadcasts telemetry.
    pub fn tick(&mut self) {
        self.on_tick(&mut NoTap);
    }

    /// [`Self::tick`], with `tap` given each telemetry sample before and
    /// after the host's corrections.
    pub fn tick_with(&mut self, tap: &mut impl TelemetryTap) {
        self.on_tick(tap);
    }

    /// The clients the engine queues messages for, in key order.
    #[must_use]
    pub fn client_keys(&self) -> Vec<ClientKey> {
        self.clients.keys()
    }

    /// Forgets a client whose connection is gone: its queue and its
    /// deduplication records.
    pub fn disconnect(&mut self, client: ClientKey) {
        self.clients.remove(client);
        self.action_dedup.forget(client);
        self.adapter_rejection_dedup.forget(client);
    }

    /// Queues `message` for `client`, as an engine reply is.
    pub fn send_message(&mut self, client: ClientKey, message: ToConnection) {
        self.send_to(client, message, MessageClass::Unicast);
    }

    /// Queues `message` for every client, as an engine broadcast is.
    pub fn broadcast_message(&mut self, message: ToConnection, class: MessageClass) {
        self.broadcast(message, class);
    }

    fn current(&self) -> MonoTimestamp {
        MonoTimestamp::from_nanos(u64::try_from(self.now.as_nanos()).unwrap_or(u64::MAX))
    }

    /// Handles one client command at the time of the current call.
    pub fn handle_command(&mut self, command: ToEngine) {
        match command {
            ToEngine::ClientConnected { client } => {
                self.clients.insert(client);
            }
            ToEngine::ClientMessage {
                client,
                message,
                now,
            } => {
                let current = self.current();
                self.on_received_message(client, *message, now, current);
            }
            ToEngine::NavGuidance { vehicle, state } => match state {
                Some(state) => {
                    self.nav_guidance.insert(vehicle, *state);
                }
                None => {
                    self.nav_guidance.remove(&vehicle);
                }
            },
        }
    }

    fn on_received_message(
        &mut self,
        client: ClientKey,
        message: DomainEnvelope,
        received: MonoTimestamp,
        current: MonoTimestamp,
    ) {
        let sampled = match &message {
            DomainEnvelope::Frame(frame) => Some(frame.sampled_at),
            DomainEnvelope::ActionCommand(command) if command.timing.is_some() => {
                Some(command.sampled_at)
            }
            _ => None,
        };
        if let Some(sampled) = sampled {
            self.engine.observe_receive_time(client, sampled, received);
        }
        self.on_client_message(client, message, current);
    }

    fn on_client_message(
        &mut self,
        client: ClientKey,
        message: DomainEnvelope,
        now: MonoTimestamp,
    ) {
        let is_disconnect = matches!(message, DomainEnvelope::Disconnect);
        let rejected_announcement = matches!(&message, DomainEnvelope::ProfileActivation(_));
        let outcome = self.engine.handle_client_message(client, message, now);
        // Evidence derives from the engine's EXPLICIT acceptance event
        // (enacted below); an announcement whose outcome carries none was
        // rejected — comparing engine state here could misreport a
        // rejected duplicate or foreign-session announcement as accepted.
        if rejected_announcement
            && !outcome
                .actions
                .iter()
                .any(|action| matches!(action, SessionAction::ActivationAccepted { .. }))
        {
            warn!(
                client = client.as_u64(),
                "control profile activation REJECTED (session or revision binding failed)"
            );
        }
        if is_disconnect {
            self.clients.remove(client);
            self.action_dedup.forget(client);
            self.adapter_rejection_dedup.forget(client);
        }
        self.enact(outcome);
    }

    fn on_tick(&mut self, tap: &mut impl TelemetryTap) {
        let now = self.current();
        let outcome = self.engine.handle_tick(now);
        self.enact(outcome);

        let step_outcome = self.adapter.step(StepBudget { ticks: 1 });
        debug!(advanced = step_outcome.advanced, "adapter stepped");

        for loss in self.adapter.take_control_authority_losses() {
            let outcome = self.engine.handle_device_authority_loss(&loss, now);
            self.enact(outcome);
        }

        self.broadcast_telemetry(now, tap);
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
                let bytes = crate::wire_codec::encode_frame_rejected_datagram(&rejection);
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
                self.action_dedup.forget(client);
                self.adapter_rejection_dedup.forget(client);
            }
            SessionAction::ActivationAccepted { client, activation } => {
                // The traceability record for control evidence (INPUT-01):
                // emitted only on the engine's explicit acceptance.
                info!(
                    client = client.as_u64(),
                    profile = %activation.profile_id,
                    profile_revision = activation.profile_revision,
                    activation_revision = activation.activation_revision,
                    digest = %hex_digest(&activation.digest),
                    device_profile = %activation.device_profile_id,
                    device_profile_revision = activation.device_profile_revision,
                    device_digest = %hex_digest(&activation.device_digest),
                    "control profile activation accepted"
                );
            }
            SessionAction::ApplyToAdapter { client, frame } => {
                self.apply_to_adapter(client, frame);
            }
            SessionAction::InvalidatePendingCommands { vehicle, scope } => {
                self.adapter.invalidate_pending_commands(vehicle, &scope);
            }
            action @ (SessionAction::EngageLinkLoss { .. }
            | SessionAction::ClearLinkLoss { .. }) => {
                self.enact_link_loss(action);
            }
        }
    }

    /// Queues `message` for `client`. In-core queues do not fill, so the
    /// class only names the message for the log.
    fn send_to(&mut self, client: ClientKey, message: ToConnection, class: MessageClass) {
        if !self.clients.send(client, message) {
            debug!(
                client = client.as_u64(),
                ?class,
                "message for a client that is not connected"
            );
        }
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

    /// Queues `message` for every connected client.
    fn broadcast(&mut self, message: ToConnection, class: MessageClass) {
        debug!(?class, "broadcast");
        self.clients.broadcast(&message);
    }
}

/// Lowercase-hex rendering of a profile content digest for the evidence log.
fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;

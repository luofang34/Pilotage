//! The client-session state machine.

use pilotage_protocol::wire;

use crate::action::{ClientAction, ClientFault, ModuleEvent};
use crate::authority::AuthorityMirror;
use crate::bootstrap;
use crate::catalog::Admission;
use crate::control::{ControlCommand, ControlLane, SCHEMA_VERSION};
use crate::event::TransportEvent;
use crate::reconnect::{ReconnectPolicy, ReconnectState};
use crate::streams::{StreamTable, drain_envelopes};

/// Construction-time settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// Name the host records for this connection.
    pub client_name: String,
    /// Backoff bounds for recovery.
    pub reconnect: ReconnectPolicy,
}

/// Where the session stands. Shells render this; they do not infer it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPhase {
    /// No transport yet, or a scheduled retry not yet attempted.
    Disconnected,
    /// Transport up, hello sent, welcome not yet received.
    AwaitingWelcome,
    /// Admitted as an observer; modules may consume inputs.
    Admitted,
    /// Stopped for good on a typed fault.
    Stopped,
}

/// The one client-session state machine (ADR-0037).
#[derive(Debug)]
pub struct ClientEngine {
    config: ClientConfig,
    phase: ClientPhase,
    streams: StreamTable,
    bootstrap_pending: Vec<u8>,
    admission: Option<Admission>,
    authority: AuthorityMirror,
    lane: Option<ControlLane>,
    pending_lease: Option<(u64, String)>,
    activation_announced: bool,
    reconnect: ReconnectState,
}

impl ClientEngine {
    /// A fresh engine in [`ClientPhase::Disconnected`].
    #[must_use]
    pub fn new(config: ClientConfig) -> Self {
        Self {
            config,
            phase: ClientPhase::Disconnected,
            streams: StreamTable::default(),
            bootstrap_pending: Vec::new(),
            admission: None,
            authority: AuthorityMirror::default(),
            lane: None,
            pending_lease: None,
            activation_announced: false,
            reconnect: ReconnectState::default(),
        }
    }

    /// The current phase.
    #[must_use]
    pub fn phase(&self) -> &ClientPhase {
        &self.phase
    }

    /// The admission catalog, once admitted.
    #[must_use]
    pub fn admission(&self) -> Option<&Admission> {
        self.admission.as_ref()
    }

    /// The authority mirror.
    #[must_use]
    pub fn authority(&self) -> &AuthorityMirror {
        &self.authority
    }

    /// Whether a control lane is open (a lease is held).
    #[must_use]
    pub fn holds_control(&self) -> bool {
        self.lane.is_some()
    }

    /// The (vehicle, scope) the open control lane is fenced to.
    #[must_use]
    pub fn control_target(&self) -> Option<(u64, String)> {
        self.lane
            .as_ref()
            .map(|lane| (lane.vehicle_id(), lane.scope().to_owned()))
    }

    /// Consumes one transport event.
    pub fn handle(&mut self, event: TransportEvent, now_ms: u64) -> Vec<ClientAction> {
        match event {
            TransportEvent::Connected => self.on_connected(),
            TransportEvent::BootstrapReceived(bytes) => self.on_bootstrap(&bytes),
            TransportEvent::UniStreamOpened(stream) => {
                self.streams.opened(stream);
                Vec::new()
            }
            TransportEvent::UniStreamReceived(stream, bytes) => {
                let output = self.streams.receive(stream, &bytes);
                let mut actions: Vec<ClientAction> = output
                    .envelopes
                    .into_iter()
                    .flat_map(|envelope| self.on_session_event(envelope))
                    .collect();
                actions.extend(
                    output
                        .video_bodies
                        .into_iter()
                        .map(|body| ClientAction::Emit(ModuleEvent::VideoFrame(body))),
                );
                actions
            }
            TransportEvent::UniStreamClosed(stream) => {
                self.streams.closed(stream);
                Vec::new()
            }
            TransportEvent::DatagramReceived(bytes) => self.on_datagram(&bytes),
            TransportEvent::TransportLost { .. } => self.on_lost(now_ms),
        }
    }

    /// Requests a lease. Only an admitted, explicitly asked-for lease is
    /// ever sent: recovery never calls this.
    pub fn request_lease(&mut self, vehicle_id: u64, scope: &str) -> Vec<ClientAction> {
        if self.phase != ClientPhase::Admitted {
            return Vec::new();
        }
        self.pending_lease = Some((vehicle_id, scope.to_owned()));
        vec![ClientAction::SendBootstrap(bootstrap::lease_request(
            vehicle_id, scope,
        ))]
    }

    /// Releases the held lease, if any.
    pub fn release_lease(&mut self) -> Vec<ClientAction> {
        let Some(lane) = self.lane.take() else {
            return Vec::new();
        };
        vec![ClientAction::SendBootstrap(bootstrap::lease_release(
            lane.vehicle_id(),
            lane.scope(),
        ))]
    }

    /// Builds and returns the next fenced control-frame datagram, or
    /// nothing when no lease is held — a shell cannot send unfenced input.
    pub fn control_frame(
        &mut self,
        command: ControlCommand,
        sampled_at_nanos: u64,
    ) -> Vec<ClientAction> {
        match self.lane.as_mut() {
            Some(lane) => vec![ClientAction::SendDatagram(
                lane.frame(command, sampled_at_nanos),
            )],
            None => Vec::new(),
        }
    }

    /// Builds a reliable discrete-action command under the held lease.
    pub fn control_action(&mut self, request: wire::ControlActionRequest) -> Vec<ClientAction> {
        match self.lane.as_mut() {
            Some(lane) => vec![ClientAction::SendBootstrap(lane.action_command(request))],
            None => Vec::new(),
        }
    }

    /// Binds the activated control profile to the lane's frames.
    pub fn bind_profile(&mut self, profile_revision: u32, activation_revision: u32) {
        if let Some(lane) = self.lane.as_mut() {
            lane.bind_profile(profile_revision, activation_revision);
        }
    }

    fn on_connected(&mut self) -> Vec<ClientAction> {
        self.phase = ClientPhase::AwaitingWelcome;
        self.bootstrap_pending.clear();
        self.streams.reset();
        vec![ClientAction::SendBootstrap(bootstrap::hello(
            &self.config.client_name,
        ))]
    }

    fn on_bootstrap(&mut self, bytes: &[u8]) -> Vec<ClientAction> {
        self.bootstrap_pending.extend_from_slice(bytes);
        let envelopes = drain_envelopes(&mut self.bootstrap_pending);
        envelopes
            .into_iter()
            .flat_map(|envelope| self.on_bootstrap_envelope(envelope))
            .collect()
    }

    fn on_bootstrap_envelope(&mut self, envelope: wire::Envelope) -> Vec<ClientAction> {
        if envelope.schema_version != SCHEMA_VERSION {
            self.phase = ClientPhase::Stopped;
            return vec![ClientAction::Stop(ClientFault::SchemaMismatch {
                host: envelope.schema_version,
                supported: SCHEMA_VERSION,
            })];
        }
        match envelope.payload {
            Some(wire::envelope::Payload::ServerWelcome(welcome)) => self.on_welcome(&welcome),
            Some(wire::envelope::Payload::LeaseResponse(response)) => self.on_lease(response),
            Some(wire::envelope::Payload::LeaseReleased(released)) => {
                vec![ClientAction::Emit(ModuleEvent::LeaseReleased(released))]
            }
            Some(wire::envelope::Payload::ControlActionResult(result)) => {
                vec![ClientAction::Emit(ModuleEvent::ActionResult(result))]
            }
            _ => Vec::new(),
        }
    }

    fn on_welcome(&mut self, welcome: &wire::ServerWelcome) -> Vec<ClientAction> {
        let Some(admission) = Admission::from_welcome(welcome) else {
            return Vec::new();
        };
        self.authority.seed(&admission.scope_holders);
        self.admission = Some(admission.clone());
        self.phase = ClientPhase::Admitted;
        self.reconnect.reset();
        vec![ClientAction::Emit(ModuleEvent::Admitted(admission))]
    }

    fn on_lease(&mut self, response: wire::LeaseResponse) -> Vec<ClientAction> {
        // The lane opens only for the grant the shell asked for: the
        // response's own (vehicle, scope) must match the pending request,
        // so a grant this client never requested cannot arm control.
        let vehicle_id = response.vehicle.as_ref().map_or(0, |v| v.value);
        let scope = response
            .scope
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let matches_pending = self
            .pending_lease
            .as_ref()
            .is_some_and(|(v, s)| *v == vehicle_id && *s == scope);
        let mut actions = Vec::new();
        if response.granted
            && matches_pending
            && let Some(admission) = self.admission.as_ref()
        {
            self.pending_lease = None;
            let generation = response.generation.as_ref().map_or(0, |g| g.value);
            let mut lane = ControlLane::new(admission.session_id, vehicle_id, scope, generation);
            // The host refuses actions and typed frames from a connection
            // that never announced its control profile; the announcement
            // travels with the first grant and the lane binds to it.
            if !self.activation_announced {
                self.activation_announced = true;
                actions.push(ClientAction::SendBootstrap(bootstrap::profile_activation(
                    admission.session_id,
                )));
            }
            lane.bind_profile(
                bootstrap::NATIVE_PROFILE_REVISION,
                bootstrap::NATIVE_ACTIVATION_REVISION,
            );
            self.lane = Some(lane);
        }
        actions.push(ClientAction::Emit(ModuleEvent::Lease(response)));
        actions
    }

    fn on_session_event(&mut self, envelope: wire::Envelope) -> Vec<ClientAction> {
        match envelope.payload {
            Some(wire::envelope::Payload::AuthorityEvent(event)) => {
                let principal = self.admission.as_ref().map_or(0, |a| a.principal_id);
                self.authority.apply(&event, principal);
                vec![ClientAction::Emit(ModuleEvent::Authority(event))]
            }
            _ => Vec::new(),
        }
    }

    fn on_datagram(&mut self, bytes: &[u8]) -> Vec<ClientAction> {
        use prost::Message;
        let Ok(envelope) = wire::Envelope::decode(bytes) else {
            // One malformed datagram proves nothing about the next one.
            return Vec::new();
        };
        match envelope.payload {
            Some(wire::envelope::Payload::TelemetrySample(sample)) => {
                vec![ClientAction::Emit(ModuleEvent::Telemetry(Box::new(sample)))]
            }
            Some(wire::envelope::Payload::Pong(pong)) => {
                vec![ClientAction::Emit(ModuleEvent::Pong(pong))]
            }
            Some(wire::envelope::Payload::FrameRejected(rejected)) => {
                vec![ClientAction::Emit(ModuleEvent::ControlRejected(rejected))]
            }
            _ => Vec::new(),
        }
    }

    fn on_lost(&mut self, now_ms: u64) -> Vec<ClientAction> {
        if self.phase == ClientPhase::Stopped {
            return Vec::new();
        }
        // Authority does not survive the loss; observation is what recovery
        // restores. The lane is dropped and never rebuilt by reconnect.
        self.lane = None;
        self.pending_lease = None;
        self.admission = None;
        self.activation_announced = false;
        self.streams.reset();
        self.bootstrap_pending.clear();
        self.phase = ClientPhase::Disconnected;
        let at_ms = self
            .reconnect
            .next_attempt_at(&self.config.reconnect, now_ms);
        vec![
            ClientAction::Emit(ModuleEvent::ConnectionDown {
                retry_at_ms: Some(at_ms),
            }),
            ClientAction::ScheduleReconnect { at_ms },
        ]
    }
}

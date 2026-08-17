//! The client-session state machine.

use std::collections::BTreeMap;

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
    pub(crate) admission: Option<Admission>,
    authority: AuthorityMirror,
    pub(crate) lanes: BTreeMap<(u64, String), ControlLane>,
    pub(crate) pending_leases: BTreeMap<(u64, String), Escalation>,
    pub(crate) pending_takeover: Option<(u64, String)>,
    pub(crate) activation_announced: bool,
    pub(crate) profile: bootstrap::ProfileIdentity,
    reconnect: ReconnectState,
}

/// What a denied lease request escalates to. A holder-denied cooperative
/// request becomes the ask; a quiet one (a runtime-planned gimbal lease)
/// only reports the denial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Escalation {
    /// Escalate a holder-present denial into a transfer ask.
    Cooperative,
    /// Surface the denial and stop.
    Quiet,
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
            lanes: BTreeMap::new(),
            pending_leases: BTreeMap::new(),
            pending_takeover: None,
            activation_announced: false,
            profile: bootstrap::ProfileIdentity::default(),
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

    /// Whether any control lane is open (some lease is held).
    #[must_use]
    pub fn holds_control(&self) -> bool {
        !self.lanes.is_empty()
    }

    /// Whether the lane for this exact (vehicle, scope) is open.
    #[must_use]
    pub fn holds(&self, vehicle_id: u64, scope: &str) -> bool {
        self.lanes.contains_key(&(vehicle_id, scope.to_owned()))
    }

    /// The (vehicle, scope) of the first open lane — the whole story for
    /// a single-lease client, and only a starting point for one holding
    /// several scopes.
    #[must_use]
    pub fn control_target(&self) -> Option<(u64, String)> {
        self.lanes.keys().next().cloned()
    }

    /// Announces the given profile identity on the first grant and binds
    /// every lane's frames to it. Set before control is requested; the
    /// announcement has already left once a lane is open.
    pub fn set_profile_identity(&mut self, profile: bootstrap::ProfileIdentity) {
        self.profile = profile;
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

    /// Requests a lease that escalates a holder-present denial into the
    /// cooperative ask. Only an admitted, explicitly asked-for lease is
    /// ever sent: recovery never calls this.
    pub fn request_lease(&mut self, vehicle_id: u64, scope: &str) -> Vec<ClientAction> {
        self.request_lease_with(vehicle_id, scope, Escalation::Cooperative)
    }

    /// Requests a lease whose denial is only reported — the path for a
    /// runtime-planned auxiliary scope (the gimbal), where asking a
    /// standing holder to hand over is not this press's intent.
    pub fn request_lease_quiet(&mut self, vehicle_id: u64, scope: &str) -> Vec<ClientAction> {
        self.request_lease_with(vehicle_id, scope, Escalation::Quiet)
    }

    fn request_lease_with(
        &mut self,
        vehicle_id: u64,
        scope: &str,
        escalation: Escalation,
    ) -> Vec<ClientAction> {
        if self.phase != ClientPhase::Admitted {
            return Vec::new();
        }
        self.pending_leases
            .insert((vehicle_id, scope.to_owned()), escalation);
        vec![ClientAction::SendBootstrap(bootstrap::lease_request(
            vehicle_id, scope,
        ))]
    }

    /// Asks the present holder to hand a scope over. The ask changes
    /// nothing until the holder offers; the engine then accepts the offer
    /// addressed to it and opens the lane on the committed transfer.
    pub fn request_takeover(&mut self, vehicle_id: u64, scope: &str) -> Vec<ClientAction> {
        if self.phase != ClientPhase::Admitted {
            return Vec::new();
        }
        self.pending_takeover = Some((vehicle_id, scope.to_owned()));
        vec![ClientAction::SendBootstrap(bootstrap::transfer_request(
            vehicle_id, scope,
        ))]
    }

    /// Offers the named held scope to another principal — the holder's
    /// half of a cooperative handover. Without that lane there is
    /// nothing to offer.
    pub fn offer_transfer(&mut self, to_principal: u64, scope: &str) -> Vec<ClientAction> {
        let Some(key) = self.lanes.keys().find(|(_, held)| held == scope).cloned() else {
            return Vec::new();
        };
        self.lanes.remove(&key);
        vec![ClientAction::SendBootstrap(bootstrap::transfer_offer(
            key.0,
            &key.1,
            to_principal,
        ))]
    }

    /// Releases the named held lease, if open.
    pub fn release_lease(&mut self, vehicle_id: u64, scope: &str) -> Vec<ClientAction> {
        let Some(lane) = self.lanes.remove(&(vehicle_id, scope.to_owned())) else {
            return Vec::new();
        };
        vec![ClientAction::SendBootstrap(bootstrap::lease_release(
            lane.vehicle_id(),
            lane.scope(),
        ))]
    }

    /// Builds the next fenced control-frame datagram on the named lane,
    /// or nothing when that lease is not held — a shell cannot send
    /// unfenced input, and a frame must never ride a sibling's fencing.
    pub fn control_frame(
        &mut self,
        vehicle_id: u64,
        scope: &str,
        command: ControlCommand,
        sampled_at_nanos: u64,
    ) -> Vec<ClientAction> {
        match self.lanes.get_mut(&(vehicle_id, scope.to_owned())) {
            Some(lane) => vec![ClientAction::SendDatagram(
                lane.frame(command, sampled_at_nanos),
            )],
            None => Vec::new(),
        }
    }

    /// Builds a reliable discrete-action command under the named lease.
    pub fn control_action(
        &mut self,
        vehicle_id: u64,
        scope: &str,
        request: wire::ControlActionRequest,
    ) -> Vec<ClientAction> {
        match self.lanes.get_mut(&(vehicle_id, scope.to_owned())) {
            Some(lane) => vec![ClientAction::SendBootstrap(lane.action_command(request))],
            None => Vec::new(),
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
        let key = (vehicle_id, scope.clone());
        let pending = self.pending_leases.get(&key).copied();
        // One operator intent, one flow: a cooperative request denied
        // because someone holds the scope becomes the ask, without a
        // second press. A quiet request only reports its denial.
        if !response.granted && pending == Some(Escalation::Cooperative) && response.reason == 1 {
            self.pending_leases.remove(&key);
            self.pending_takeover = Some((vehicle_id, scope.clone()));
            return vec![
                ClientAction::SendBootstrap(bootstrap::transfer_request(vehicle_id, &scope)),
                ClientAction::Emit(ModuleEvent::Lease(response)),
            ];
        }
        let mut actions = Vec::new();
        if response.granted && pending.is_some() {
            self.pending_leases.remove(&key);
            let generation = response.generation.as_ref().map_or(0, |g| g.value);
            actions = self.open_lane(vehicle_id, scope, generation);
        }
        actions.push(ClientAction::Emit(ModuleEvent::Lease(response)));
        actions
    }

    /// Opens (or re-fences) the lane for a granted scope: the activation
    /// announcement travels with the first grant, and every lane binds
    /// its frames to that announced identity. The host refuses actions
    /// and typed frames from a connection that never announced one.
    pub(crate) fn open_lane(
        &mut self,
        vehicle_id: u64,
        scope: String,
        generation: u64,
    ) -> Vec<ClientAction> {
        let Some(admission) = self.admission.as_ref() else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        let mut lane =
            ControlLane::new(admission.session_id, vehicle_id, scope.clone(), generation);
        if !self.activation_announced {
            self.activation_announced = true;
            actions.push(ClientAction::SendBootstrap(bootstrap::profile_activation(
                admission.session_id,
                &self.profile,
            )));
        }
        lane.bind_profile(
            self.profile.profile_revision,
            self.profile.activation_revision,
        );
        self.lanes.insert((vehicle_id, scope), lane);
        actions
    }

    fn on_session_event(&mut self, envelope: wire::Envelope) -> Vec<ClientAction> {
        match envelope.payload {
            Some(wire::envelope::Payload::AuthorityEvent(event)) => {
                let principal = self.admission.as_ref().map_or(0, |a| a.principal_id);
                self.authority.apply(&event, principal);
                let mut actions = self.on_transfer_progress(&event, principal);
                actions.push(ClientAction::Emit(ModuleEvent::Authority(event)));
                actions
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
        // restores. The lanes are dropped and never rebuilt by reconnect.
        self.lanes.clear();
        self.pending_leases.clear();
        self.pending_takeover = None;
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

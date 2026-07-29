//! The mission principal's task loop: registration with the engine
//! actor, the tick that turns mission-engine output into typed frames
//! and reliable action commands, and the exit record. The session flow
//! and authority fencing live in the submodules.

mod fencing;
mod nav_guidance;
mod session_flow;

use std::collections::HashMap;
use std::time::Duration;

use navigate_contract::MonotonicNanos;
use pilotage_mission::{MissionAction, MissionEngine, MissionEvent, NavGuidance};
use pilotage_protocol::{
    ClientHello, ControlActionCommand, ControlIntent, ControlPayload, Generation, PrincipalId,
    SESSION_PROTOCOL_VERSION, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
};
use pilotage_session::DomainEnvelope;
use pilotage_timing::MonoTimestamp;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tracing::{info, warn};

use crate::runtime::HOST_VEHICLE;
use crate::runtime::connection::ToConnection;
use crate::runtime::engine_actor::ToEngine;

use super::ownship;
use super::{AutomationStatus, MISSION_CLIENT, MissionPlan};
use nav_guidance::{NavGuidancePublisher, NavPublication};

/// The scope the mission principal leases and frames against.
const MISSION_SCOPE: &str = "vehicle.motion";

/// The announced control-profile identity (INPUT-01 traceability).
const PROFILE_ID: &str = "automation.mission";

/// The profile document's own revision.
const PROFILE_REVISION: u32 = 1;

/// The principal's single activation revision: one profile, installed
/// once per session, so the monotonic revision never advances.
const ACTIVATION_REVISION: u32 = 1;

/// Stable content digest for the automation profile announcement. The
/// engine binds session and monotonic revision only, but the digest must
/// be stable so evidence records agree across runs.
const PROFILE_DIGEST: [u8; 32] = *b"pilotage.automation.mission.v1\0\0";

/// No device profile is selected; the id stays empty and the digest zero.
const NO_DEVICE_DIGEST: [u8; 32] = [0; 32];

/// Mission tick cadence. Matches the `MissionConfig::frame_interval`
/// default and sits well inside the holder-silence watchdog window.
const TICK_INTERVAL: Duration = Duration::from_millis(50);

/// The in-process mission principal's state.
pub(super) struct MissionTask {
    engine: mpsc::WeakSender<ToEngine>,
    start: Instant,
    plan: Option<MissionPlan>,
    status: watch::Sender<AutomationStatus>,
    session: Option<SessionId>,
    principal: Option<PrincipalId>,
    generation: Option<Generation>,
    fenced: bool,
    mission: Option<MissionEngine>,
    sequence: u32,
    next_action_id: u32,
    pending_actions: HashMap<u32, u64>,
    /// Stamps the guidance group; bound to the session at the welcome,
    /// since the incarnation token it carries is derived from it.
    nav_guidance: Option<NavGuidancePublisher>,
}

impl MissionTask {
    pub(super) fn new(
        engine: mpsc::WeakSender<ToEngine>,
        start: Instant,
        plan: MissionPlan,
        status: watch::Sender<AutomationStatus>,
    ) -> Self {
        Self {
            engine,
            start,
            plan: Some(plan),
            status,
            session: None,
            principal: None,
            generation: None,
            fenced: false,
            mission: None,
            sequence: 0,
            next_action_id: 0,
            pending_actions: HashMap::new(),
            nav_guidance: None,
        }
    }

    /// Registers with the engine actor, opens the session, and services
    /// engine replies and the mission tick until the actor goes away or
    /// the engine closes the connection.
    pub(super) async fn run(
        mut self,
        outbound_tx: mpsc::Sender<ToConnection>,
        mut outbound_rx: mpsc::Receiver<ToConnection>,
    ) {
        let connected = self
            .send_command(ToEngine::ClientConnected {
                client: MISSION_CLIENT,
                sender: outbound_tx,
            })
            .await;
        if !connected {
            return;
        }
        let hello = DomainEnvelope::Hello(ClientHello {
            protocol_version: SESSION_PROTOCOL_VERSION,
            client_name: "mission-executor".to_owned(),
            join_token: Vec::new(),
        });
        if !self.send_message(hello).await {
            return;
        }
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                message = outbound_rx.recv() => match message {
                    Some(message) => {
                        if !self.on_connection_message(message).await {
                            break;
                        }
                    }
                    None => break,
                },
                _ = ticker.tick() => {
                    if !self.on_tick().await {
                        break;
                    }
                }
            }
        }
        // A principal that stops flying stops guiding: clear the group so
        // no instrument keeps a frozen leg on screen after it is gone.
        let nanos = self.elapsed_nanos();
        self.publish_nav_guidance(None, nanos).await;
        self.log_counters();
    }

    /// Handles one engine-actor delivery; `false` stops the task.
    async fn on_connection_message(&mut self, message: ToConnection) -> bool {
        match message {
            ToConnection::BootstrapMessage { bytes, .. } => self.on_bootstrap(&bytes).await,
            ToConnection::AuthorityMessage(bytes) => {
                self.on_authority(&bytes);
                true
            }
            ToConnection::Datagram { bytes, .. } => {
                self.on_datagram(&bytes);
                true
            }
            ToConnection::Close => {
                warn!("engine closed the mission principal's connection");
                self.update(|status| status.closed = true);
                false
            }
        }
    }

    /// One mission tick: events are logged, an intent becomes a typed
    /// frame, an action becomes a reliable command. Never runs while
    /// fenced — human takeover wins and the task only holds (ADR-0025).
    async fn on_tick(&mut self) -> bool {
        if self.fenced || self.generation.is_none() {
            return true;
        }
        let nanos = self.elapsed_nanos();
        let output = {
            let Some(mission) = self.mission.as_mut() else {
                return true;
            };
            mission.tick(MonotonicNanos::from_nanos(nanos))
        };
        self.log_events(&output.events);
        self.update(|status| status.mission_state = Some(output.state));
        let guidance = self.mission.as_ref().and_then(MissionEngine::nav_guidance);
        if !self.publish_nav_guidance(guidance.as_ref(), nanos).await {
            return false;
        }
        if let Some(action) = output.action
            && !self.send_action(action).await
        {
            return false;
        }
        if let Some(intent) = output.intent
            && !self
                .send_intent(intent, MonoTimestamp::from_nanos(nanos))
                .await
        {
            return false;
        }
        true
    }

    /// Hands this tick's guidance to the telemetry assembly, or clears
    /// what it holds once the executor stops flying a leg (ADR-0031):
    /// display context travels as its own stamped group, and absence —
    /// never zeros — is how "no guidance" reaches an instrument.
    async fn publish_nav_guidance(
        &mut self,
        guidance: Option<&NavGuidance>,
        acquired_at_ns: u64,
    ) -> bool {
        let Some(publisher) = self.nav_guidance.as_mut() else {
            return true;
        };
        let Some(publication) = publisher.publication(guidance, acquired_at_ns) else {
            return true;
        };
        let state = match publication {
            NavPublication::Sample(state) => Some(Box::new(state)),
            NavPublication::Clear => None,
        };
        self.send_command(ToEngine::NavGuidance {
            vehicle: HOST_VEHICLE,
            state,
        })
        .await
    }

    /// Frames one typed intent under the held lease with a wrap-advancing
    /// sequence and an exact host-clock sample stamp.
    async fn send_intent(&mut self, intent: ControlIntent, sampled_at: MonoTimestamp) -> bool {
        let (Some(session), Some(generation)) = (self.session, self.generation) else {
            return true;
        };
        self.sequence = self.sequence.wrapping_add(1);
        let frame = ScopedControlFrame {
            session,
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new(MISSION_SCOPE),
            generation,
            sequence: SequenceNum::new(self.sequence),
            sampled_at,
            profile_revision: PROFILE_REVISION,
            activation_revision: ACTIVATION_REVISION,
            payload: ControlPayload::default(),
            intent: Some(intent),
            actions: Vec::new(),
            action_ids: Vec::new(),
        };
        if !self
            .send_message_at(DomainEnvelope::Frame(frame), sampled_at)
            .await
        {
            return false;
        }
        self.update(|status| status.frames_sent = status.frames_sent.wrapping_add(1));
        true
    }

    /// Sends one discrete action as a reliable `ControlActionCommand`
    /// under a fresh nonzero wire correlation id, remembering the mission
    /// engine's own id for the answering result.
    async fn send_action(&mut self, action: MissionAction) -> bool {
        let (Some(session), Some(generation)) = (self.session, self.generation) else {
            return true;
        };
        self.next_action_id = self.next_action_id.wrapping_add(1);
        if self.next_action_id == 0 {
            // The wire reserves zero for "no correlation"; skip it on wrap.
            self.next_action_id = 1;
        }
        let wire_id = self.next_action_id;
        self.pending_actions.insert(wire_id, action.action_id);
        info!(action = ?action.action, action_id = wire_id, "mission action command");
        let command = ControlActionCommand {
            session,
            vehicle: HOST_VEHICLE,
            scope: ScopeId::new(MISSION_SCOPE),
            generation,
            activation_revision: ACTIVATION_REVISION,
            action: action.action,
            action_id: wire_id,
        };
        self.send_message(DomainEnvelope::ActionCommand(command))
            .await
    }

    fn log_events(&self, events: &[MissionEvent]) {
        for event in events {
            match event {
                MissionEvent::GuidanceRefused { reason } => {
                    warn!(?reason, "mission guidance refused");
                }
                MissionEvent::MissionComplete => info!("mission complete"),
                other => info!(event = ?other, "mission event"),
            }
        }
    }

    /// The exit record: every named mission counter plus the frame
    /// bookkeeping, so a run's refusals are visible even without a
    /// status watcher.
    fn log_counters(&self) {
        let status = self.status.borrow().clone();
        let Some(mission) = self.mission.as_ref() else {
            info!("mission principal exiting before the engine was built");
            return;
        };
        let counters = mission.counters();
        info!(
            state = ?mission.state(),
            rejected_role = counters.rejected_role,
            fusion_rejected = counters.fusion_rejected,
            guidance_refused = counters.guidance_refused,
            arm_rejected = counters.arm_rejected,
            frames_sent = status.frames_sent,
            frames_rejected = status.frames_rejected,
            "mission principal exiting"
        );
    }

    fn elapsed_nanos(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn mission_now(&self) -> MonotonicNanos {
        MonotonicNanos::from_nanos(self.elapsed_nanos())
    }

    fn host_now(&self) -> MonoTimestamp {
        MonoTimestamp::from_nanos(self.elapsed_nanos())
    }

    /// Sends one command to the engine actor through the weak handle;
    /// `false` means the actor is gone and the task should exit.
    async fn send_command(&self, command: ToEngine) -> bool {
        let Some(sender) = self.engine.upgrade() else {
            return false;
        };
        sender.send(command).await.is_ok()
    }

    async fn send_message(&self, message: DomainEnvelope) -> bool {
        self.send_message_at(message, self.host_now()).await
    }

    async fn send_message_at(&self, message: DomainEnvelope, now: MonoTimestamp) -> bool {
        self.send_command(ToEngine::ClientMessage {
            client: MISSION_CLIENT,
            message,
            now,
        })
        .await
    }

    fn update(&self, apply: impl FnOnce(&mut AutomationStatus)) {
        self.status.send_modify(apply);
    }
}

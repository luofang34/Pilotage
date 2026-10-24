//! The mission principal's task loop: registration with the engine
//! actor, the tick that turns mission-engine output into typed frames
//! and reliable action commands, and the exit record. The session flow
//! and authority fencing live in the submodules.

mod fencing;
mod flight_watch;
mod nav_guidance;
mod run_control;
mod session_flow;
#[cfg(test)]
mod test_support;

pub(crate) use run_control::RunControl;

use std::collections::HashMap;
use std::time::Duration;

use navigate_contract::MonotonicNanos;
use pilotage_mission::{
    ExecutionTarget, MissionAction, MissionEngine, MissionEvent, NavGuidance, OwnshipSource,
};
use pilotage_protocol::{
    ClientHello, ControlActionCommand, ControlIntent, ControlPayload, Generation, PrincipalId,
    ReferenceFrame, SESSION_PROTOCOL_VERSION, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
    VelocityIntent,
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
use pilotage_session::ClientKey;

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
    /// Whether this host's UNSTAMPED planar poses are the simulator's
    /// own state (true only for the deterministic reference adapter).
    /// Anything else must arrive under a stamped role or be refused.
    planar_pose_is_truth: bool,
    /// Which telemetry lane becomes the mission's ownship.
    ownship_source: OwnshipSource,
    /// The plan targets a real vehicle its pilot launches.
    real_vehicle: bool,
    /// A human holds the motion scope; the mission waits for it back.
    held: bool,
    /// Frames still owed as neutral input under a new generation.
    neutral_frames: u8,
    /// A real vehicle's mission waits for the pilot's launch before it
    /// asks for the motion lease.
    awaiting_launch: bool,
    /// The lease request the next tick sends.
    lease_due: bool,
    /// Once airborne, a real vehicle's mission keeps asking for the motion
    /// lease whenever it is held: a human who took or holds the scope
    /// gives it back by releasing it.
    lease_wanted: bool,
    /// Mission-clock time of the next lease request while held.
    lease_retry_at: Option<u64>,
    /// The key this principal registers under.
    client: ClientKey,
    /// Hold, resume and abort from the mission lane.
    controls: Option<mpsc::Receiver<RunControl>>,
    /// An operator's hold: the vehicle is kept stopped in place.
    operator_hold: bool,
    /// Neutral frames still owed before an abort releases the lease.
    stopping: Option<u8>,
    /// The flight controller's reports during this run.
    flight: flight_watch::FlightWatch,
}

impl MissionTask {
    pub(super) fn new(
        engine: mpsc::WeakSender<ToEngine>,
        start: Instant,
        plan: MissionPlan,
        status: watch::Sender<AutomationStatus>,
        planar_pose_is_truth: bool,
    ) -> Self {
        let config = plan.config();
        let ownship_source = config.ownship_source;
        let real_vehicle = config.execution_target == ExecutionTarget::RealVehicle;
        Self {
            real_vehicle,
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
            planar_pose_is_truth,
            ownship_source,
            held: false,
            neutral_frames: 0,
            awaiting_launch: false,
            lease_due: false,
            lease_wanted: false,
            lease_retry_at: None,
            client: MISSION_CLIENT,
            controls: None,
            operator_hold: false,
            stopping: None,
            flight: flight_watch::FlightWatch::default(),
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
                client: self.client,
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
        let mut controls = self.controls.take();
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
                control = run_control::next(&mut controls) => self.on_control(control),
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
            ToConnection::AuthorityMessage(bytes) => match self.on_authority(&bytes) {
                Some(reply) => self.send_message(reply).await,
                None => true,
            },
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
    /// frame, an action becomes a reliable command. A fenced task does
    /// nothing. A held task ticks the engine so its ownship and status
    /// stay fresh, and sends nothing: the human holds the scope
    /// (ADR-0025, ADR-0051).
    async fn on_tick(&mut self) -> bool {
        if self.fenced {
            return true;
        }
        let now = self.elapsed_nanos();
        if self.lease_due || self.lease_retry_due(now) {
            self.lease_due = false;
            self.lease_retry_at = None;
            if !self.request_lease().await {
                return false;
            }
        }
        if self.generation.is_none() && !self.held {
            return true;
        }
        if self.operator_hold || self.stopping.is_some() {
            return self.hold_tick().await;
        }
        let nanos = self.elapsed_nanos();
        let output = {
            let Some(mission) = self.mission.as_mut() else {
                return true;
            };
            mission.tick(MonotonicNanos::from_nanos(nanos))
        };
        self.log_events(&output.events);
        let progress = self.mission.as_ref().map(MissionEngine::plan_progress);
        self.update(|status| {
            status.mission_state = Some(output.state);
            if let Some((target, complete)) = progress {
                status.target_waypoint = Some(target);
                status.plan_complete = complete;
            }
        });
        if self.held {
            return true;
        }
        let guidance = self.mission.as_ref().and_then(MissionEngine::nav_guidance);
        if !self.publish_nav_guidance(guidance.as_ref(), nanos).await {
            return false;
        }
        if output.action.is_some()
            && self.neutral_frames > 0
            && !self
                .send_intent(neutral_intent(), MonoTimestamp::from_nanos(nanos))
                .await
        {
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
    /// sequence and an exact host-clock sample stamp. The first frames
    /// after a resume are neutral: a vehicle that requires neutral input
    /// on a new lease admits the new generation on them.
    async fn send_intent(&mut self, intent: ControlIntent, sampled_at: MonoTimestamp) -> bool {
        let (Some(session), Some(generation)) = (self.session, self.generation) else {
            return true;
        };
        let intent = if self.neutral_frames > 0 {
            self.neutral_frames = self.neutral_frames.saturating_sub(1);
            neutral_intent()
        } else {
            intent
        };
        self.sequence = self.sequence.wrapping_add(1);
        let frame = ScopedControlFrame {
            timing: None,
            admitted_timing: None,
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
            sampled_at: pilotage_timing::MonoTimestamp::from_nanos(0),
            timing: None,
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

    /// Logs the tick's events and folds its task events into the status, the lane's
    /// record of which task the run flies.
    fn log_events(&self, events: &[MissionEvent]) {
        for event in events {
            match event {
                MissionEvent::GuidanceRefused { reason } => {
                    warn!(?reason, "mission guidance refused");
                }
                MissionEvent::TaskStarted { task } => {
                    info!(%task, "task started");
                    self.update(|status| status.active_task = Some(task.clone()));
                }
                MissionEvent::TaskFinished { task } => {
                    info!(%task, "task finished");
                    self.update(|status| {
                        status.active_task = None;
                        status.finished_tasks.push(task.clone());
                    });
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

    pub(super) fn mission_now(&self) -> MonotonicNanos {
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
            client: self.client,
            message: Box::new(message),
            now,
        })
        .await
    }

    fn update(&self, apply: impl FnOnce(&mut AutomationStatus)) {
        self.status.send_modify(apply);
    }
}

/// Zero velocity: the frame that demonstrates neutral to the session.
fn neutral_intent() -> ControlIntent {
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    })
}

//! The in-process mission principal (ADR-0025, ADR-0030): a local
//! automation client that speaks the session vocabulary verbatim —
//! hello, profile activation, lease, typed intent frames, reliable
//! action commands — against the same session engine remote operators
//! reach, so it is fenced by exactly the same authority rules. In the
//! core it is a state machine the core steps: its commands queue for the
//! engine, and the engine's replies come back on the same call.

pub mod ownship;
mod task;

pub use task::{MissionTask, RunControl, TICK_INTERVAL};

use aerocontext_core::NavDataSnapshot;
use navigate_contract::ClockDomainId;
use pilotage_mission::{MissionConfig, MissionState, SnapshotProvenance, SurveyRequest};
use pilotage_session::ClientKey;

/// The mission principal's client key, from the collision-free end of the
/// key space as in the session host.
pub const MISSION_CLIENT: ClientKey = ClientKey::new(u64::MAX);

/// The single clock domain every mission `now`, ownship stamp, and fusion
/// judgment lives on: the core's origin (ADR-0009).
pub const MISSION_CLOCK: ClockDomainId = ClockDomainId::new(1);

/// Observable progress of the in-process mission principal, published on
/// a watch channel for logs and the integration test.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutomationStatus {
    /// Session id assigned by the welcome, once the handshake completed.
    pub session: Option<u64>,
    /// The control-profile activation announcement has been sent.
    pub activation_sent: bool,
    /// Fencing generation of the held motion lease.
    pub lease_generation: Option<u64>,
    /// The engine closed this principal's connection, or the mission is
    /// otherwise ended for this task; nothing resumes it.
    pub fenced: bool,
    /// A human holds the motion scope: the mission stopped framing and
    /// waits for a transfer back (ADR-0051).
    pub held: bool,
    /// The mission lost a lease it held. Only a transfer offered to this
    /// principal, the hand-back, returns control; no other run of the lane
    /// may take the lease meanwhile (ADR-0051).
    pub awaiting_hand_back: bool,
    /// The principal that took the lease from a run that waits for a
    /// hand-back released it, or its connection closed, without a hand-back:
    /// no one holds the lease, and the approval's recovery may apply.
    pub control_freed: bool,
    /// A real vehicle's mission is waiting for its pilot to launch it.
    pub awaiting_launch: bool,
    /// A real vehicle's mission is waiting for the vehicle to advertise
    /// velocity control.
    pub awaiting_capabilities: bool,
    /// Lease requests scheduled while another principal held the scope.
    pub lease_retries: u32,
    /// Transfers back that resumed the mission.
    pub resumes: u32,
    /// Offers this principal made to humans who requested the scope.
    pub offers: u32,
    /// The vehicle accepted the arm action.
    pub arm_accepted: bool,
    /// Current mission phase, once the engine is built.
    pub mission_state: Option<MissionState>,
    /// Typed intent frames submitted to the session engine.
    pub frames_sent: u64,
    /// The running digest of the wire bytes of every intent frame sent,
    /// in order: two runs that send the same frames end with the same
    /// digest.
    pub frame_digest: [u8; 32],
    /// `FrameRejected` notices received back for submitted frames.
    pub frames_rejected: u64,
    /// The engine closed this principal's connection.
    pub closed: bool,
    /// Index of the waypoint the plan flies toward, once the engine is
    /// built.
    pub target_waypoint: Option<usize>,
    /// The plan's last waypoint is captured.
    pub plan_complete: bool,
    /// An operator's hold keeps the vehicle stopped in place.
    pub operator_held: bool,
    /// An abort stopped the vehicle, released the lease and ended the run.
    pub stopped: bool,
    /// The vehicle disarmed or landed on its own during a flown phase, and
    /// the run ended for that reason.
    pub ended_by_vehicle: Option<&'static str>,
    /// An operator's hold or an abort holds the vehicle, and the vehicle
    /// reports that it is still.
    pub holding: bool,
    /// The principal that holds the motion lease when another one does, as
    /// the welcome or an authority event named it.
    pub lease_holder: Option<u64>,
    /// The task the run flies now, as the mission core names its phase.
    pub active_task: Option<String>,
    /// The tasks the run finished, in order.
    pub finished_tasks: Vec<String>,
    /// What the operations of the finished tasks returned, by task.
    pub results: Vec<(String, Vec<pilotage_protocol::wire::MissionValue>)>,
    /// The tasks whose operation a cancel stopped before its end.
    pub cancelled_tasks: Vec<String>,
    /// The tasks whose operation failed.
    pub failed_tasks: Vec<String>,
    /// The questions the run's operations ask a person now, by task.
    pub questions: Vec<(String, pilotage_protocol::wire::MissionQuestion)>,
    /// The run of a plan of operations ended before its end, for this
    /// reason.
    pub ended_because: Option<&'static str>,
}

/// Where the plan comes from.
pub enum PlanSource {
    /// A route that expands against a navigation-data snapshot, with the
    /// snapshot's provenance (ADR-0030).
    Route {
        /// The decoded snapshot.
        snapshot: NavDataSnapshot,
        /// The blob-verified provenance record.
        provenance: SnapshotProvenance,
    },
    /// The survey request the plan is derived from.
    Survey(SurveyRequest),
    /// Home from wherever the vehicle is, at the cruise height.
    Home,
    /// A waypoint list.
    Waypoints(pilotage_mission::WaypointPlan),
}

/// A validated mission: the plan source and the config the flight engine
/// is built from once capabilities are known.
pub struct MissionPlan {
    source: PlanSource,
    config: MissionConfig,
}

impl MissionPlan {
    /// A plan from `source` under `config`.
    #[must_use]
    pub fn new(source: PlanSource, config: MissionConfig) -> Self {
        Self { source, config }
    }

    /// A waypoint plan under `config`.
    #[must_use]
    pub fn waypoints(plan: pilotage_mission::WaypointPlan, config: MissionConfig) -> Self {
        Self {
            source: PlanSource::Waypoints(plan),
            config,
        }
    }

    /// This plan, with the engine holding the arm until the vehicle states
    /// on its telemetry that it is ready to arm. For a host whose flight
    /// controller states its readiness (`FcState.ready_to_arm`): one that
    /// never states it would never arm.
    #[must_use]
    pub fn arming_when_vehicle_ready(mut self) -> Self {
        self.config.arm_when_vehicle_ready = true;
        self
    }

    /// The mission config over this plan's route and anchor; limits are
    /// tightened by the caller from the advertised capabilities.
    #[must_use]
    pub fn config(&self) -> MissionConfig {
        self.config.clone()
    }
}

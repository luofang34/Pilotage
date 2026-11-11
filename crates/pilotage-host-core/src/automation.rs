//! The in-process mission principal (ADR-0025, ADR-0030): a local
//! automation client that speaks the session vocabulary verbatim —
//! hello, profile activation, lease, typed intent frames, reliable
//! action commands — against the same session engine remote operators
//! reach, so it is fenced by exactly the same authority rules. In the
//! core it is a state machine the core steps: its commands queue for the
//! engine, and the engine's replies come back on the same call.

pub(crate) mod ownship;
mod task;

pub(crate) use task::{MissionTask, RunControl, TICK_INTERVAL as MISSION_TICK};

use navigate_contract::ClockDomainId;
use pilotage_mission::{MissionConfig, MissionState, SurveyRequest};
use pilotage_session::ClientKey;

/// The mission principal's client key, from the collision-free end of the
/// key space as in the session host.
const MISSION_CLIENT: ClientKey = ClientKey::new(u64::MAX);

/// The single clock domain every mission `now`, ownship stamp, and fusion
/// judgment lives on: the core's origin (ADR-0009).
pub const MISSION_CLOCK: ClockDomainId = ClockDomainId::new(1);

/// Observable progress of the in-process mission principal, published on
/// a watch channel for logs and the integration test.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AutomationStatus {
    /// Session id assigned by the welcome, once the handshake completed.
    pub(crate) session: Option<u64>,
    /// The control-profile activation announcement has been sent.
    pub(crate) activation_sent: bool,
    /// Fencing generation of the held motion lease.
    pub(crate) lease_generation: Option<u64>,
    /// The engine closed this principal's connection, or the mission is
    /// otherwise ended for this task; nothing resumes it.
    pub(crate) fenced: bool,
    /// A human holds the motion scope: the mission stopped framing and
    /// waits for a transfer back (ADR-0051).
    pub(crate) held: bool,
    /// The mission lost a lease it held. Only a transfer offered to this
    /// principal, the hand-back, returns control; no other run of the lane
    /// may take the lease meanwhile (ADR-0051).
    pub(crate) awaiting_hand_back: bool,
    /// The principal that took the lease from a run that waits for a
    /// hand-back released it, or its connection closed, without a hand-back:
    /// no one holds the lease, and the approval's recovery may apply.
    pub(crate) control_freed: bool,
    /// A real vehicle's mission is waiting for its pilot to launch it.
    pub(crate) awaiting_launch: bool,
    /// A real vehicle's mission is waiting for the vehicle to advertise
    /// velocity control.
    pub(crate) awaiting_capabilities: bool,
    /// Lease requests scheduled while another principal held the scope.
    pub(crate) lease_retries: u32,
    /// Transfers back that resumed the mission.
    pub(crate) resumes: u32,
    /// Offers this principal made to humans who requested the scope.
    pub(crate) offers: u32,
    /// The vehicle accepted the arm action.
    pub(crate) arm_accepted: bool,
    /// Current mission phase, once the engine is built.
    pub(crate) mission_state: Option<MissionState>,
    /// Typed intent frames submitted to the session engine.
    pub(crate) frames_sent: u64,
    /// The running digest of the wire bytes of every intent frame sent,
    /// in order: two runs that send the same frames end with the same
    /// digest.
    pub(crate) frame_digest: [u8; 32],
    /// `FrameRejected` notices received back for submitted frames.
    pub(crate) frames_rejected: u64,
    /// The engine closed this principal's connection.
    pub(crate) closed: bool,
    /// Index of the waypoint the plan flies toward, once the engine is
    /// built.
    pub(crate) target_waypoint: Option<usize>,
    /// The plan's last waypoint is captured.
    pub(crate) plan_complete: bool,
    /// An operator's hold keeps the vehicle stopped in place.
    pub(crate) operator_held: bool,
    /// An abort stopped the vehicle, released the lease and ended the run.
    pub(crate) stopped: bool,
    /// The vehicle disarmed or landed on its own during a flown phase, and
    /// the run ended for that reason.
    pub(crate) ended_by_vehicle: Option<&'static str>,
    /// An operator's hold or an abort holds the vehicle, and the vehicle
    /// reports that it is still.
    pub(crate) holding: bool,
    /// The principal that holds the motion lease when another one does, as
    /// the welcome or an authority event named it.
    pub(crate) lease_holder: Option<u64>,
    /// The task the run flies now, as the mission core names its phase.
    pub(crate) active_task: Option<String>,
    /// The tasks the run finished, in order.
    pub(crate) finished_tasks: Vec<String>,
    /// What the operations of the finished tasks returned, by task.
    pub(crate) results: Vec<(String, Vec<pilotage_protocol::wire::MissionValue>)>,
    /// The tasks whose operation a cancel stopped before its end.
    pub(crate) cancelled_tasks: Vec<String>,
    /// The tasks whose operation failed.
    pub(crate) failed_tasks: Vec<String>,
    /// The questions the run's operations ask a person now, by task.
    pub(crate) questions: Vec<(String, pilotage_protocol::wire::MissionQuestion)>,
    /// The run of a plan of operations ended before its end, for this
    /// reason.
    pub(crate) ended_because: Option<&'static str>,
}

/// Where the plan comes from. A route expands against navigation data,
/// which an embedding host does not carry; the core flies the plans that
/// need none.
#[allow(dead_code)]
pub(crate) enum PlanSource {
    /// The survey request the plan is derived from.
    Survey(SurveyRequest),
    /// Home from wherever the vehicle is, at the cruise height.
    Home,
    /// A waypoint list.
    Waypoints(pilotage_mission::WaypointPlan),
}

/// A validated mission: the plan source and the config the flight engine
/// is built from once capabilities are known.
pub(crate) struct MissionPlan {
    source: PlanSource,
    config: MissionConfig,
}

impl MissionPlan {
    /// A waypoint plan under `config`.
    pub(crate) fn waypoints(plan: pilotage_mission::WaypointPlan, config: MissionConfig) -> Self {
        Self {
            source: PlanSource::Waypoints(plan),
            config,
        }
    }

    /// The mission config over this plan's route and anchor; limits are
    /// tightened by the caller from the advertised capabilities.
    fn config(&self) -> MissionConfig {
        self.config.clone()
    }
}

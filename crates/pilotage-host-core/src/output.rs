//! What the core gives back: when to call it again, what happened, and
//! where it stands.

use navigate_contract::{MonotonicNanos, NavigationSolution};
use pilotage_mission::{MissionEvent, MissionState, NavigationDisplay};

/// When the host calls [`crate::HostCore::tick`] next, nanoseconds since
/// the core's origin. A host may call earlier; it should not call later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextWake {
    /// The time of the next tick.
    pub at: MonotonicNanos,
}

/// One thing that happened in a tick, in order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HostEvent {
    /// A mission event.
    Mission(MissionEvent),
    /// The flight controller confirmed the arm the mission asked for. The
    /// mission event that carries it follows.
    ArmConfirmed,
    /// The flight controller refused the arm the mission asked for, or did
    /// not answer in time; the mission asks again under its retry limit.
    /// The mission event that carries it follows.
    ArmRefused,
    /// The session engine refused an intent frame of the mission.
    FrameRejected,
}

/// Where the core stands after its last call.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct HostStatus {
    /// The mission phase, once a mission is loaded.
    pub mission_state: Option<MissionState>,
    /// The solution the mission flies, if it has one.
    pub estimate: Option<NavigationSolution>,
    /// The flight controller confirmed the mission's arm.
    pub armed: bool,
    /// Whether the plan's last waypoint is done.
    pub plan_complete: bool,
    /// What a navigation display shows of the mission, once its engine is
    /// built: the plan's waypoints with their progress, the guidance of
    /// the leg flown now, and the holding pattern a vehicle that cannot
    /// stop flies after its plan. A part with nothing to show is absent.
    pub navigation: Option<NavigationDisplay>,
    /// Intent frames the mission submitted to the session engine.
    pub frames_sent: u64,
    /// Frames the session engine refused.
    pub frames_rejected: u64,
    /// SHA-256 chained over the wire bytes of every intent frame sent,
    /// in order: two runs that send the same frames end equal.
    pub frame_digest: [u8; 32],
    /// MAVLink frames handed to the host for the flight controller.
    pub uplink_frames: u64,
}

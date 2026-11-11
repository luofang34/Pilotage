//! What the core gives back: when to call it again, what happened, and
//! where it stands.

use navigate_contract::{MonotonicNanos, NavigationSolution};
use pilotage_mission::{MissionEvent, MissionState};

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
    /// The flight controller confirmed the arm the mission asked for.
    ArmConfirmed,
    /// The flight controller refused the arm, or did not answer in time.
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
    /// Whether the plan's last waypoint is done.
    pub plan_complete: bool,
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

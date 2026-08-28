//! Typed outputs of one engine tick: intent, action, state, events,
//! and the refusal counters.

use navigate_fpl::SequenceReason;
use navigate_guidance::GuidanceRefusal;
use pilotage_protocol::{ControlAction, ControlIntent};

/// The mission phase the engine is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissionState {
    /// Waiting for the fusion filter to publish a first solution.
    AwaitSolution,
    /// Arm requested; waiting for the correlated action result.
    Arming,
    /// Climbing to the cruise height above the anchor.
    Climb,
    /// Flying the plan under velocity guidance.
    Enroute,
    /// The plan is complete; every tick commands zero velocity so the
    /// adapter's brake-then-hold takes over.
    Complete,
    /// The core refused or stopped the mission before plan completion.
    Failed,
}

/// A discrete action the host must send as a `ControlActionCommand` on
/// the reliable stream, with this engine-chosen correlation id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissionAction {
    /// The typed action.
    pub action: ControlAction,
    /// Nonzero correlation id; the host reports the correlated result
    /// back via [`crate::MissionEngine::on_action_result`].
    pub action_id: u64,
}

/// One observable mission event surfaced by a tick.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MissionEvent {
    /// An arm action was emitted with this correlation id.
    ArmRequested {
        /// Correlation id of the emitted action.
        action_id: u64,
    },
    /// The vehicle accepted the arm action.
    ArmAccepted {
        /// Correlation id of the accepted action.
        action_id: u64,
    },
    /// The vehicle rejected the arm action; the next tick re-sends with
    /// a fresh id.
    ArmRejected {
        /// Correlation id of the rejected action.
        action_id: u64,
    },
    /// The climb phase began.
    ClimbStarted,
    /// Enroute guidance began.
    EnrouteStarted,
    /// A waypoint captured and the next leg is active.
    LegAdvanced {
        /// Index of the newly active `to` waypoint in fly order.
        to_index: usize,
        /// Which sequencing rule authorized the advance: turn
        /// anticipation ahead of a fly-by fix, or the capture radius.
        /// Telemetry carries it so an early transition is attributable
        /// to the rule rather than read as a skipped waypoint.
        reason: SequenceReason,
    },
    /// The final waypoint captured; emitted exactly once.
    MissionComplete,
    /// The sequencing core stopped the mission with a typed result.
    MissionFailed {
        /// The typed core terminal result.
        result: pilotage_mission_core::MissionTerminal,
    },
    /// The sequencing core refused one host input.
    MissionEngineRefused {
        /// The core error detail for the operational log.
        detail: String,
    },
    /// Guidance refused to issue a setpoint this tick; no intent was
    /// emitted (the host's silence watchdog is the backstop). Repeats of
    /// the same refusal kind are counted but not re-surfaced until the
    /// kind changes or guidance succeeds again.
    GuidanceRefused {
        /// The typed refusal.
        reason: GuidanceRefusal,
    },
}

/// What one call to [`crate::MissionEngine::tick`] produced.
#[derive(Debug, Clone, PartialEq)]
pub struct MissionOutput {
    /// The typed intent to frame this tick, when the engine has one.
    pub intent: Option<ControlIntent>,
    /// A discrete action to send, when one is due.
    pub action: Option<MissionAction>,
    /// The phase after this tick.
    pub state: MissionState,
    /// Events surfaced by this tick, in occurrence order.
    pub events: Vec<MissionEvent>,
}

/// Named refusal/rejection counters; every refusal increments exactly
/// one. Counters wrap rather than saturate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MissionCounters {
    /// Ownship samples refused for a non-truth source role.
    pub rejected_role: u64,
    /// Synthesized observations the fusion filter refused (its own
    /// per-reason counters live on the filter).
    pub fusion_rejected: u64,
    /// Ticks on which guidance refused to issue a setpoint.
    pub guidance_refused: u64,
    /// Arm actions the vehicle rejected.
    pub arm_rejected: u64,
    /// Ticks that produced a guided velocity but no intent because no
    /// sample has carried a heading yet (the NED→body rotation needs
    /// one; zero would steer toward due north).
    pub missing_yaw: u64,
}

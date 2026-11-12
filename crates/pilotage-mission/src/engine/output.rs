//! Typed outputs of one engine tick: intent, action, state, events,
//! and the refusal counters.

use navigate_guidance::GuidanceRefusal;
use navigate_route::SequenceReason;
use pilotage_protocol::{ControlAction, ControlIntent};

use super::LateralMode;

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
    /// A human holds the motion scope. The mission commands nothing, keeps
    /// its ownship fresh, and pauses its sequencing clock until the host
    /// resumes it.
    Held,
    /// The plan is complete. A vehicle that can stop is commanded to zero
    /// velocity on every tick, so the adapter's brake-then-hold takes
    /// over; one that cannot stop flies its holding pattern.
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
    /// The last waypoint of the plan sequenced.
    LastWaypointSequenced {
        /// The sequencing rule: the capture radius, or a waypoint passed
        /// outside it with the distance by which it was missed.
        reason: SequenceReason,
        /// The horizontal distance to the last waypoint at the sequence,
        /// metres.
        distance_m: f64,
    },
    /// A vehicle that cannot stop completed its plan with no further
    /// instruction, and now flies a holding pattern at the last waypoint.
    HoldingPatternStarted {
        /// The pattern.
        pattern: crate::HoldingPattern,
    },
    /// The vehicle reached the holding fix and started this entry.
    HoldingPatternEntered {
        /// The entry the course at the fix selected.
        entry: crate::HoldingEntry,
    },
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
    /// A waypoint sequenced and the next leg is active.
    LegAdvanced {
        /// Index of the newly active `to` waypoint in fly order.
        to_index: usize,
        /// Which sequencing rule authorized the advance: turn
        /// anticipation ahead of a fly-by fix, the capture radius, or a
        /// fix passed outside the capture radius (with the distance by
        /// which it was missed). Telemetry carries it so a transition is
        /// attributable to the rule rather than read as a skipped
        /// waypoint.
        reason: SequenceReason,
    },
    /// The mission core entered the phase of a task.
    TaskStarted {
        /// The task's phase identifier.
        task: String,
    },
    /// The mission core completed the phase of a task.
    TaskFinished {
        /// The task's phase identifier.
        task: String,
    },
    /// The final waypoint captured; emitted exactly once.
    MissionComplete,
    /// The host held the mission: a human took the motion scope.
    MissionHeld,
    /// The host resumed the mission after a hold of this length.
    MissionResumed {
        /// How long the mission was held, nanoseconds on the engine clock.
        held_ns: u64,
    },
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
    /// The engine holds back the start of the mission, and why. Emitted
    /// each time a start is held; the start goes ahead, announced by its
    /// own event, once the reason clears.
    StartWithheld {
        /// Why the start is held.
        reason: super::StartWithheldReason,
    },
    /// The vehicle is held because no solution is fit to fly in flight, and
    /// why: a fixed wing on the onboard navigator holds the heading and
    /// altitude it last commanded, a vehicle that can stop holds its
    /// position on its flight controller's own estimate. Emitted when the
    /// hold begins and when its reason changes.
    NavigationHold {
        /// Why no solution is fit to fly.
        reason: super::StartWithheldReason,
    },
    /// The engine's estimate of the flight controller's heading bias moved
    /// a degree or more from the value last reported: the heading the
    /// flight controller holds is this far right of the true heading it is
    /// commanded, and every command is turned against it. Near zero on a
    /// healthy flight controller.
    HeadingBiasEstimated {
        /// The bias, radians, positive right.
        bias_rad: f64,
    },
    /// The plan goes on by dead reckoning: in flight, the onboard solution
    /// is beyond its limits (graded unusable, or with a horizontal
    /// uncertainty above the limit) and is flown all the same under
    /// [`crate::BeyondLimits::DeadReckon`]. Emitted when the dead
    /// reckoning begins and each time its horizontal uncertainty doubles.
    DeadReckoning {
        /// The 1-sigma horizontal uncertainty flown on, meters.
        horizontal_1sigma_m: f64,
        /// The 1-sigma vertical uncertainty flown on, meters.
        vertical_1sigma_m: f64,
    },
    /// A solution within its limits is flown again after dead reckoning.
    NavigationRecovered {
        /// The 1-sigma horizontal uncertainty of that solution, meters.
        horizontal_1sigma_m: f64,
    },
    /// The vehicle passed the active fix outside its capture radius, and
    /// guidance now flies direct back to it. Emitted once per fix.
    FixOverflown {
        /// Index of the missed `to` waypoint in fly order.
        to_index: usize,
    },
    /// The heading hold of a vehicle with no position to fly on ran its
    /// bound, and a racetrack flown by heading and time began where the
    /// vehicle was.
    TimedHoldStarted {
        /// The racetrack's inbound course, radians clockwise from true
        /// north: the heading held.
        inbound_course_rad: f64,
        /// Its inbound leg time, seconds.
        leg_time_s: f64,
    },
    /// The wind estimate that corrects the racetrack flown by time is
    /// older than the configured age. The racetrack keeps it; where the
    /// racetrack lies over the ground is an assumption from now on.
    TimedHoldWindStale {
        /// The estimate's age, seconds.
        age_s: f64,
    },
    /// A position came back during a heading hold or a racetrack flown by
    /// time, and this lateral mode is flown again.
    LateralResumed {
        /// The mode resumed.
        mode: LateralMode,
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
    /// Ticks under the vehicle-estimate source on which the latest
    /// sample was too old to fly, so no solution was published.
    pub estimate_stale: u64,
    /// Ticks flown direct back to a fix the vehicle passed without
    /// capturing it.
    pub fix_overflown: u64,
    /// Onboard navigation solutions refused when offered: on another
    /// ellipsoid, with an implausible position or a non-finite velocity,
    /// or stamped later than the offer.
    pub onboard_refused: u64,
    /// Onboard navigation solutions offered again, or older than the held
    /// one. They are not new solutions and do not renew the held one's
    /// age.
    pub onboard_repeated: u64,
    /// Ticks under the onboard-navigation source on which the held
    /// solution was too old to fly, so no solution was published.
    pub onboard_stale: u64,
    /// Ticks under the onboard-navigation source on which the held
    /// solution was graded unusable or had a horizontal uncertainty above
    /// the configured limit, so no solution was published.
    pub onboard_unfit: u64,
    /// Ticks under the onboard-navigation source on which the held
    /// solution was beyond its limits and flown by dead reckoning.
    pub onboard_dead_reckoned: u64,
    /// Ticks in the climb on which the vertical mode machine and the
    /// mission core, which ends its climb phase by the mission document's
    /// own threshold, disagreed on whether the climb's target is
    /// captured. Any count is a defect.
    pub vertical_mode_mismatch: u64,
    /// Times the lifecycle machine's reported state differed from the
    /// state the engine derives from the mission core and the hold. Any
    /// count is a defect.
    pub lifecycle_mismatch: u64,
}

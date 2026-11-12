//! The vertical mode machine: whether the vehicle climbs, descends, or
//! holds an altitude, and toward which one.
//!
//! Two pure functions. [`relate`] turns an altitude, a target, and a
//! capture margin into where the vehicle is relative to the target.
//! [`next`] turns a mode and one event into the next mode and whether the
//! target was captured. Neither reads the mission's phase: the target is
//! handed in, so no vertical behaviour is implied by another machine's
//! state.
//!
//! A capture is one-sided and it holds: a climb is captured at or above
//! the target less the margin, a descent at or below the target plus it,
//! and a captured target stays captured until a new target is set. A
//! vehicle that sags out of the margin is still holding that altitude,
//! not climbing to it again.
//!
//! The mode drives what the engine itself decides: the climb's vertical
//! command and the mode a display reads. The mission core still ends its
//! climb phase by the altitude threshold in the mission document, with
//! the same margin and altitude. The engine counts every tick on which
//! the machine and the core disagree
//! ([`super::MissionCounters::vertical_mode_mismatch`]), so the two
//! judges cannot drift apart unseen.

use navigate_contract::{MonotonicNanos, NavigationSolution};
use pilotage_mission_core::FlightAction as CoreFlightAction;

use super::document::climb_capture_margin_m;
use super::{MissionEngine, MissionState};

#[cfg(test)]
mod tests;

/// The vertical mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VerticalMode {
    /// No altitude is flown to.
    #[default]
    Off,
    /// A target is set and the vehicle's altitude is not known.
    Pending,
    /// The vehicle climbs to the target.
    Climb,
    /// The vehicle descends to the target.
    Descend,
    /// The target is captured and held.
    Hold,
}

/// Where the vehicle is relative to a target altitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Relation {
    /// Below the target by more than the margin.
    Below,
    /// Within the margin of the target.
    Within,
    /// Above the target by more than the margin.
    Above,
    /// The altitude is not known.
    Unknown,
}

/// What one tick found of the vertical target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum VerticalEvent {
    /// No altitude is flown to any more.
    Disengaged,
    /// A target was set, or changed, with the vehicle so related to it.
    TargetSet(Relation),
    /// The target stands, with the vehicle so related to it.
    Sample(Relation),
}

/// What a transition gives to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum VerticalEffect {
    /// Nothing.
    None,
    /// The target was captured on this event.
    Captured,
}

/// Where `altitude_m` is relative to `target_m` with a capture margin of
/// `margin_m`. An altitude or a target that is not a number is unknown.
pub(super) fn relate(altitude_m: Option<f64>, target_m: f64, margin_m: f64) -> Relation {
    let Some(altitude_m) = altitude_m else {
        return Relation::Unknown;
    };
    let above_m = altitude_m - target_m;
    if above_m.is_nan() {
        Relation::Unknown
    } else if above_m < -margin_m {
        Relation::Below
    } else if above_m > margin_m {
        Relation::Above
    } else {
        Relation::Within
    }
}

/// The mode after `event` in `mode`, and whether the target was captured.
pub(super) const fn next(
    mode: VerticalMode,
    event: VerticalEvent,
) -> (VerticalMode, VerticalEffect) {
    use Relation::{Above, Below, Unknown, Within};
    use VerticalMode::{Climb, Descend, Hold, Off, Pending};
    let to = match (mode, event) {
        (_, VerticalEvent::Disengaged) => Off,
        (_, VerticalEvent::TargetSet(Below)) => Climb,
        (_, VerticalEvent::TargetSet(Above)) => Descend,
        (_, VerticalEvent::TargetSet(Within)) => Hold,
        (_, VerticalEvent::TargetSet(Unknown)) => Pending,
        // A sample with no target says nothing.
        (Off, VerticalEvent::Sample(_)) => Off,
        (Pending, VerticalEvent::Sample(Below)) => Climb,
        (Pending, VerticalEvent::Sample(Above)) => Descend,
        (Pending, VerticalEvent::Sample(Within)) => Hold,
        (Pending, VerticalEvent::Sample(Unknown)) => Pending,
        (Climb, VerticalEvent::Sample(Below | Unknown)) => Climb,
        (Climb, VerticalEvent::Sample(Within | Above)) => Hold,
        (Descend, VerticalEvent::Sample(Above | Unknown)) => Descend,
        (Descend, VerticalEvent::Sample(Within | Below)) => Hold,
        (Hold, VerticalEvent::Sample(_)) => Hold,
    };
    let effect = match (mode, to) {
        (Hold, Hold) => VerticalEffect::None,
        (_, Hold) => VerticalEffect::Captured,
        _ => VerticalEffect::None,
    };
    (to, effect)
}

/// The vertical mode with the altitude it flies to.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VerticalStatus {
    /// The mode after the latest tick.
    pub mode: VerticalMode,
    /// The target altitude above the ellipsoid, metres; `None` in
    /// [`VerticalMode::Off`].
    pub target_m: Option<f64>,
}

impl MissionEngine {
    /// The vertical mode and its target, from the state the latest
    /// [`MissionEngine::tick`] left.
    #[must_use]
    pub const fn vertical_status(&self) -> VerticalStatus {
        self.modes.vertical
    }

    /// The vertical mode after the latest step.
    pub(super) const fn vertical_mode(&self) -> VerticalMode {
        self.modes.vertical.mode
    }

    /// Steps the machine for a tick in `state` at `now`. A step with an
    /// unchanged target and altitude changes nothing, so a tick may step
    /// again once its phase is known.
    pub(super) fn step_vertical(
        &mut self,
        state: MissionState,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
    ) {
        let target_m = self.vertical_target_m(state);
        let event = match target_m {
            None => VerticalEvent::Disengaged,
            Some(target_m) => {
                let relation = relate(
                    self.observed_altitude_m(solution, now),
                    target_m,
                    climb_capture_margin_m(&self.config),
                );
                if self.modes.vertical.target_m == Some(target_m) {
                    VerticalEvent::Sample(relation)
                } else {
                    VerticalEvent::TargetSet(relation)
                }
            }
        };
        let (mode, _) = next(self.modes.vertical.mode, event);
        self.modes.vertical = VerticalStatus { mode, target_m };
    }

    /// Counts a disagreement between the machine and the mission core on
    /// a tick that began in the climb: the machine holds the climb's
    /// target exactly when the core has left the climb for the route.
    pub(super) fn compare_vertical(&mut self, before: MissionState, after: MissionState) {
        let left_climb = match (before, after) {
            (MissionState::Climb, MissionState::Climb) => false,
            (MissionState::Climb, MissionState::Enroute) => true,
            _ => return,
        };
        if (self.modes.vertical.mode == VerticalMode::Hold) != left_climb {
            self.counters.vertical_mode_mismatch =
                self.counters.vertical_mode_mismatch.wrapping_add(1);
        }
    }

    /// The altitude the mission flies to in `state`, metres above the
    /// ellipsoid: the cruise altitude from the arm request, the climb's
    /// target, the active leg's altitude, and after the plan the altitude
    /// already flown. The mission's phase is read here once, to hand the
    /// machine its target.
    fn vertical_target_m(&self, state: MissionState) -> Option<f64> {
        match state {
            MissionState::Arming
                if self.arm_request_sent() && self.config.cruise_height_m > 0.0 =>
            {
                Some(self.config.anchor.altitude_m + self.config.cruise_height_m)
            }
            MissionState::Climb => match self.active_action.as_ref() {
                Some(CoreFlightAction::Climb { target_altitude_m }) => Some(*target_altitude_m),
                _ => None,
            },
            MissionState::Enroute => self
                .route
                .active_leg()
                .map(|leg| leg.to.position.altitude_m)
                .or(self.modes.vertical.target_m),
            MissionState::Complete | MissionState::AfterPlan => self.modes.vertical.target_m,
            MissionState::AwaitSolution
            | MissionState::Arming
            | MissionState::Held
            | MissionState::Failed => None,
        }
    }
}

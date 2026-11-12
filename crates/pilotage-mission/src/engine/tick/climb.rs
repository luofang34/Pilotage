//! The climb: the vertical demand toward the climb's target, and what is
//! streamed from the arm request until the climb begins.

use navigate_contract::{MonotonicNanos, NavigationSolution};
use pilotage_mission_core::FlightAction as CoreFlightAction;
use pilotage_protocol::{ControlIntent, ReferenceFrame, VelocityIntent};

use crate::body_frame::clamp_symmetric;

use super::super::document::{CLIMB_CAPTURE_MARGIN_M, climb_capture_margin_m};
use super::super::{MissionEngine, MissionState};
use super::outputs;

impl MissionEngine {
    pub(super) fn climb_intent(
        &self,
        solution: Option<&NavigationSolution>,
    ) -> Option<ControlIntent> {
        let Some(CoreFlightAction::Climb { target_altitude_m }) = self.active_action.as_ref()
        else {
            return None;
        };
        climb_toward(&self.config, solution?, *target_altitude_m)
    }

    /// The climb of a vehicle that cannot stop: the plan's lateral
    /// guidance, with the climb toward the target as its vertical rate
    /// until the vehicle is within the capture margin of it. Without
    /// lateral guidance (none refused, none yet) the vertical climb alone
    /// is flown, on the present course.
    pub(super) fn climb_along_route_intent(
        &mut self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<super::super::MissionEvent>,
    ) -> Option<ControlIntent> {
        let climb = self.climb_intent(solution);
        let Some(ControlIntent::Velocity(mut lateral)) =
            self.follow_plan_intent(solution, now, events)
        else {
            return climb;
        };
        if let Some(ControlIntent::Velocity(vertical)) = climb {
            lateral.vz = vertical.vz;
        }
        Some(ControlIntent::Velocity(lateral))
    }

    /// The intent streamed from the arm request on, so the flight
    /// controller has a command the moment it arms instead of none until
    /// the arm's confirmation comes back: the climb that follows the arm,
    /// or, with no climb to fly, the hold of the present course and height.
    pub(super) fn arming_intent(
        &self,
        solution: Option<&NavigationSolution>,
    ) -> Option<ControlIntent> {
        let target_altitude_m = self.config.anchor.altitude_m + self.config.cruise_height_m;
        if self.config.cruise_height_m > 0.0
            && let Some(climb) = climb_toward(&self.config, solution?, target_altitude_m)
        {
            return Some(climb);
        }
        // A hold with no known heading would be a body-frame command with
        // nothing to rotate it by; the stream waits for a heading.
        self.last_yaw_rad?;
        Some(outputs::zero_velocity_intent())
    }

    /// Whether the climb is captured at `now`, by the altitude the core
    /// judges it on ([`Self::observed_altitude_m`]).
    pub(super) fn climb_is_captured(
        &self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
    ) -> bool {
        let Some(CoreFlightAction::Climb { target_altitude_m }) = self.active_action.as_ref()
        else {
            return false;
        };
        self.observed_altitude_m(solution, now)
            .is_some_and(|altitude_m| {
                altitude_m >= *target_altitude_m - climb_capture_margin_m(&self.config)
            })
    }

    /// The altitude the mission core judges the phases by at `now`: the
    /// solution's, except in the climb of a vehicle that holds its
    /// altitude itself. That vehicle's flight controller climbs to the
    /// commanded altitude on its own air data, so the climb is judged by
    /// its altitude (its height on the origin datum the commands are
    /// turned by) while a fresh one is known, and otherwise by the
    /// solution's, credited with twice its vertical uncertainty past the
    /// capture window so a navigator's honest vertical error cannot keep
    /// the mission in the climb.
    pub(in crate::engine) fn observed_altitude_m(
        &self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
    ) -> Option<f64> {
        let solution_altitude_m = solution.map(|value| value.position.altitude_m);
        if self.config.can_stop() || self.state() != MissionState::Climb {
            return solution_altitude_m;
        }
        if let (Some(origin_m), Some(height_m)) =
            (self.config.fc_origin_height_m, self.fc_height_m(now))
        {
            return Some(origin_m + height_m);
        }
        let solution = solution?;
        let window_m = climb_capture_margin_m(&self.config);
        let excess_m = (2.0 * solution.integrity.vertical_1sigma_m - window_m).max(0.0);
        Some(solution.position.altitude_m + excess_m)
    }
}

/// The climb toward `target_altitude_m`, or `None` once the vehicle is
/// within the capture margin of it.
pub(super) fn climb_toward(
    config: &crate::MissionConfig,
    solution: &NavigationSolution,
    target_altitude_m: f64,
) -> Option<ControlIntent> {
    if solution.position.altitude_m >= target_altitude_m - CLIMB_CAPTURE_MARGIN_M {
        return None;
    }
    let up_mps = clamp_symmetric(config.climb_rate_mps, config.limits.max_vertical_mps);
    Some(ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: -up_mps as f32,
        yaw_rate: 0.0,
    }))
}

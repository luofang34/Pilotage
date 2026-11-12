//! The climb: the vertical demand toward the climb's target, and what is
//! streamed from the arm request until the climb begins.

use navigate_contract::{MonotonicNanos, NavigationSolution};
use pilotage_mission_core::FlightAction as CoreFlightAction;
use pilotage_protocol::{ControlIntent, ReferenceFrame, VelocityIntent};

use crate::body_frame::clamp_symmetric;

use super::super::document::climb_capture_margin_m;
use super::super::{MissionEngine, VerticalMode};
use super::outputs;

impl MissionEngine {
    /// The climb's vertical demand: the climb rate while the vertical
    /// mode climbs to the climb's target, and none once it is captured.
    pub(in crate::engine) fn climb_intent(
        &self,
        solution: Option<&NavigationSolution>,
    ) -> Option<ControlIntent> {
        let Some(CoreFlightAction::Climb { .. }) = self.active_action.as_ref() else {
            return None;
        };
        solution?;
        (self.vertical_mode() == VerticalMode::Climb).then(|| climb_rate(&self.config))
    }

    /// The climb of a vehicle that cannot stop: the plan's lateral
    /// guidance, with the climb toward the target as its vertical rate
    /// until the vehicle is within the capture margin of it. Without
    /// lateral guidance (none refused, none yet) the vertical climb alone
    /// is flown, on the present course.
    pub(in crate::engine) fn climb_along_route_intent(
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
    pub(in crate::engine) fn arming_intent(
        &self,
        solution: Option<&NavigationSolution>,
    ) -> Option<ControlIntent> {
        if self.config.cruise_height_m > 0.0 {
            solution?;
            if self.vertical_mode() == VerticalMode::Climb {
                return Some(climb_rate(&self.config));
            }
        }
        // A hold with no known heading would be a body-frame command with
        // nothing to rotate it by; the stream waits for a heading.
        self.last_yaw_rad?;
        Some(outputs::zero_velocity_intent())
    }

    /// The altitude a vertical target is judged against at `now`: the
    /// solution's, except while a vehicle that holds its altitude itself
    /// is `climbing` to a target. That vehicle's flight controller climbs
    /// to the commanded altitude on its own air data, so its climb is
    /// judged by its altitude (its height on the origin datum the commands
    /// are turned by) while a fresh one is known, and otherwise by the
    /// solution's, credited with twice its vertical uncertainty past the
    /// capture window so a navigator's honest vertical error cannot keep
    /// the target from being captured.
    pub(in crate::engine) fn target_altitude_source_m(
        &self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        climbing: bool,
    ) -> Option<f64> {
        let solution_altitude_m = solution.map(|value| value.position.altitude_m);
        if self.config.can_stop() || !climbing {
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

/// The climb at the configured rate, within the vertical ceiling.
fn climb_rate(config: &crate::MissionConfig) -> ControlIntent {
    let up_mps = clamp_symmetric(config.climb_rate_mps, config.limits.max_vertical_mps);
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: -up_mps as f32,
        yaw_rate: 0.0,
    })
}

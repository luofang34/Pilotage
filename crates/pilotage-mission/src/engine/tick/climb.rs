//! The climb: the vertical demand toward the climb's target, and what is
//! streamed from the arm request until the climb begins.

use navigate_contract::NavigationSolution;
use pilotage_mission_core::FlightAction as CoreFlightAction;
use pilotage_protocol::{ControlIntent, ReferenceFrame, VelocityIntent};

use crate::body_frame::clamp_symmetric;

use super::super::MissionEngine;
use super::super::document::CLIMB_CAPTURE_MARGIN_M;
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

    pub(super) fn climb_is_captured(&self, solution: Option<&NavigationSolution>) -> bool {
        let Some(solution) = solution else {
            return false;
        };
        let Some(CoreFlightAction::Climb { target_altitude_m }) = self.active_action.as_ref()
        else {
            return false;
        };
        solution.position.altitude_m >= *target_altitude_m - CLIMB_CAPTURE_MARGIN_M
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

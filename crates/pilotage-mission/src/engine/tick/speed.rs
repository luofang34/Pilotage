//! The speeds a route is flown and sized at: the cruise the guidance
//! commands, and the speed over the ground that sizes the turn ahead of a
//! fly-by waypoint.

use navigate_contract::NavigationSolution;
use pilotage_protocol::ControlIntent;

use super::super::MissionEngine;

impl MissionEngine {
    /// The cruise speed guidance commands, within the horizontal ceiling.
    pub(in crate::engine) fn commanded_cruise_mps(&self) -> f64 {
        self.config
            .cruise_mps
            .min(self.config.limits.max_horizontal_mps)
    }

    /// The speed over the ground that sizes the turn ahead of a fly-by
    /// waypoint: the solution's, which carries the wind and the thin air
    /// at height. A solution that states no horizontal speed falls back to
    /// the commanded speed, as true airspeed for a fixed wing, whose
    /// command is an indicated airspeed.
    pub(super) fn route_groundspeed_mps(&self, solution: &NavigationSolution) -> f64 {
        let velocity = &solution.velocity;
        let measured = velocity.north_mps.hypot(velocity.east_mps);
        if measured.is_finite() && measured > 0.0 {
            return measured;
        }
        let commanded = self.commanded_cruise_mps();
        if self.config.can_stop() {
            commanded
        } else {
            crate::true_airspeed_for(commanded, solution.position.altitude_m)
        }
    }
}

/// A fixed wing's velocity intent with its horizontal speed at most the
/// cruise airspeed, its direction kept. A fixed-wing flight controller
/// reads the horizontal speed as the airspeed to hold: a cross-track
/// correction added to the along-track speed would speed the aircraft up
/// in every intercept, and a faster aircraft turns wider than the turn the
/// route was sized for. A slower demand, such as a waypoint's speed
/// constraint, is kept.
pub(super) fn at_most_cruise(intent: ControlIntent, cruise_mps: f64) -> ControlIntent {
    let ControlIntent::Velocity(mut velocity) = intent else {
        return intent;
    };
    let speed = f64::from(velocity.vx).hypot(f64::from(velocity.vy));
    if speed > cruise_mps && speed.is_finite() && cruise_mps.is_finite() {
        let scale = cruise_mps / speed;
        velocity.vx = (f64::from(velocity.vx) * scale) as f32;
        velocity.vy = (f64::from(velocity.vy) * scale) as f32;
    }
    ControlIntent::Velocity(velocity)
}

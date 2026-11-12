//! The speeds a route is flown and sized at: the cruise the guidance
//! commands, and the speed over the ground that sizes the turn ahead of a
//! fly-by waypoint.

use navigate_contract::NavigationSolution;
use pilotage_protocol::ControlIntent;

use super::super::MissionEngine;
use super::wind_turn;

impl MissionEngine {
    /// The cruise speed guidance commands, within the horizontal ceiling,
    /// and within the holding speed limit in a holding pattern.
    pub(in crate::engine) fn commanded_cruise_mps(&self) -> f64 {
        let cruise_mps = self
            .config
            .cruise_mps
            .min(self.config.limits.max_horizontal_mps);
        self.holding_speed_limit_mps()
            .map_or(cruise_mps, |limit_mps| cruise_mps.min(limit_mps))
    }

    /// The speed over the ground that sizes the turn ahead of a fly-by
    /// waypoint: the solution's, which carries the wind and the thin air
    /// at height. A fixed wing that knows the wind sizes the turn as it
    /// flies in that wind instead: a turn that ends downwind speeds up as
    /// it goes, and a turn sized for the speed it starts at swings wide of
    /// the next leg. A solution that
    /// states no horizontal speed falls back to the commanded speed, as
    /// true airspeed for a fixed wing, whose command is an indicated
    /// airspeed.
    pub(super) fn route_groundspeed_mps(&self, solution: &NavigationSolution) -> f64 {
        let velocity = &solution.velocity;
        let measured = velocity.north_mps.hypot(velocity.east_mps);
        if measured.is_finite() && measured > 0.0 {
            return self
                .turn_groundspeed_mps(solution.position.altitude_m)
                .unwrap_or(measured);
        }
        let commanded = self.commanded_cruise_mps();
        if self.config.can_stop() {
            commanded
        } else {
            crate::true_airspeed_for(commanded, solution.position.altitude_m)
        }
    }
}

impl MissionEngine {
    /// The speed over the ground that sizes the turn ahead the way the
    /// turn flies in the wind, m/s, for a fixed wing that knows the wind,
    /// at `altitude_m`: the speed of the still-air turn at the route's bank
    /// that starts as far before the waypoint.
    ///
    /// On heading demands the aircraft turns through the air at the cruise
    /// airspeed, and the wind carries the turn downwind. On velocity
    /// demands the engine makes good the cruise speed over the ground on
    /// every course, at the airspeed that takes, up to its ceiling; the
    /// turn then needs the most bank where the airspeed is highest, and is
    /// sized there.
    fn turn_groundspeed_mps(&self, altitude_m: f64) -> Option<f64> {
        if self.config.can_stop() {
            return None;
        }
        let wind = self.wind_estimate()?;
        let (into, out) = self.route.turn_ahead_courses()?;
        let bank_rad = self.route.bank_limit_rad();
        let cruise_mps = crate::true_airspeed_for(self.commanded_cruise_mps(), altitude_m);
        if !self.steers_by_heading() {
            let ceiling_mps =
                crate::true_airspeed_for(self.config.limits.max_horizontal_mps, altitude_m);
            return made_good_turn_groundspeed_mps(into, out, cruise_mps, ceiling_mps, wind);
        }
        let lead_m = wind_turn::lead_m(into, out, cruise_mps, wind, bank_rad)?;
        let turn = (out - into).sin().atan2((out - into).cos()).abs();
        let tan_half = (turn / 2.0).tan();
        let lateral_mps2 = crate::body::SURFACE_GRAVITY_MPS2 * bank_rad.tan();
        (tan_half > 1e-6 && lateral_mps2 > 0.0).then(|| (lead_m * lateral_mps2 / tan_half).sqrt())
    }
}

/// The step at which a turn's courses are sampled, radians (5°).
const TURN_SAMPLE_RAD: f64 = 0.087_266_462_599_716_48;

/// The speed over the ground that sizes a turn from the course `into` to
/// `out` flown on velocity demands, m/s: the ground speed `ground_mps` is
/// made good at the airspeed it takes in `wind`, up to `ceiling_mps`. A
/// turn over the ground at a ground speed `v` and an airspeed `a` needs
/// the bank of a still-air turn at `√(v a)`, so the turn is sized at the
/// greatest of these along its courses.
fn made_good_turn_groundspeed_mps(
    into: f64,
    out: f64,
    ground_mps: f64,
    ceiling_mps: f64,
    wind: [f64; 2],
) -> Option<f64> {
    let turn = (out - into).sin().atan2((out - into).cos());
    let steps = (turn.abs() / TURN_SAMPLE_RAD).ceil().max(1.0) as u32;
    (0..=steps)
        .map(|step| into + turn * f64::from(step) / f64::from(steps))
        .filter_map(|course| {
            let (sin, cos) = course.sin_cos();
            let air = (ground_mps * cos - wind[0]).hypot(ground_mps * sin - wind[1]);
            if air <= ceiling_mps {
                return Some((ground_mps * air).sqrt());
            }
            let along = wind[0] * cos + wind[1] * sin;
            let across = -wind[0] * sin + wind[1] * cos;
            let still = ceiling_mps * ceiling_mps - across * across;
            (still >= 0.0).then(|| ((along + still.sqrt()) * ceiling_mps).sqrt())
        })
        .reduce(f64::max)
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

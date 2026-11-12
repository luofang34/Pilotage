//! A fixed wing with Aviate's C172 lateral law (`aviate-core`
//! `control::fixed_wing::lateral`): a heading or course error over the
//! course time constant is a turn rate, with an integrator that trims
//! straight flight, limited to the route bank. Its flight controller's
//! heading and ground-course references carry a bias, as a magnetic
//! reference or an estimator error does, and it flies in a steady wind.

use pilotage_mission::true_airspeed_for;
use pilotage_protocol::ControlIntent;

use super::{ALTITUDE_M, STEP_S};

const GRAVITY_MPS2: f64 = 9.806_65;
/// Aviate C172 `route_bank_angle`, `course_time_constant`,
/// `course_integral_time`, `roll_time_constant`, and `max_roll_rate`.
const ROUTE_BANK_RAD: f64 = 0.44;
const COURSE_TIME_CONSTANT_S: f64 = 3.0;
const COURSE_INTEGRAL_TIME_S: f64 = 20.0;
const ROLL_TIME_CONSTANT_S: f64 = 0.5;
const MAX_ROLL_RATE_RPS: f64 = 0.5;
/// How fast the airspeed follows its command, m/s²: a C172 at cruise
/// power gains a few knots a second.
const AIRSPEED_RATE_MPS2: f64 = 1.0;
const MIN_IAS_MPS: f64 = 33.0;
const MAX_IAS_MPS: f64 = 65.0;

/// A fixed wing in the anchor's NED plane.
pub struct Aircraft {
    pub ned: [f64; 2],
    /// The true heading, radians.
    pub heading_rad: f64,
    bank_rad: f64,
    indicated_mps: f64,
    turn_rate_integrator: f64,
    /// The wind, north and east m/s.
    wind_mps: [f64; 2],
    /// How far the flight controller's heading and course references read
    /// right of the true ones, radians: it holds a commanded heading or
    /// course that much left of it.
    reference_bias_rad: f64,
    /// The last demand: a heading, or a course with an airspeed.
    demand: Option<Demand>,
    /// The indicated airspeed commanded, m/s.
    commanded_ias_mps: f64,
}

#[derive(Clone, Copy)]
enum Demand {
    Heading(f64),
    Course(f64),
}

impl Aircraft {
    pub fn new(ned: [f64; 2], heading_rad: f64, wind_mps: [f64; 2], bias_rad: f64) -> Self {
        Self {
            ned,
            heading_rad,
            bank_rad: 0.0,
            indicated_mps: 55.0,
            turn_rate_integrator: 0.0,
            wind_mps,
            reference_bias_rad: bias_rad,
            demand: None,
            commanded_ias_mps: 55.0,
        }
    }

    fn airspeed_mps(&self) -> f64 {
        true_airspeed_for(self.indicated_mps, ALTITUDE_M)
    }

    pub fn ground_velocity(&self) -> [f64; 2] {
        let airspeed = self.airspeed_mps();
        [
            airspeed * self.heading_rad.cos() + self.wind_mps[0],
            airspeed * self.heading_rad.sin() + self.wind_mps[1],
        ]
    }

    /// Takes `intent` as the demand, and flies one step of the last one.
    pub fn fly(&mut self, intent: Option<&ControlIntent>) {
        match intent {
            Some(ControlIntent::HeadingAltitude(demand)) => {
                self.demand = Some(Demand::Heading(f64::from(demand.heading_rad)));
                if let Some(airspeed) = demand.airspeed_mps {
                    self.commanded_ias_mps = f64::from(airspeed);
                }
            }
            Some(ControlIntent::Velocity(v)) => {
                let (sin, cos) = self.heading_rad.sin_cos();
                let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
                let [north, east] = [vx * cos - vy * sin, vx * sin + vy * cos];
                if north.hypot(east) > 0.0 {
                    self.demand = Some(Demand::Course(east.atan2(north)));
                    self.commanded_ias_mps = north.hypot(east).clamp(MIN_IAS_MPS, MAX_IAS_MPS);
                }
            }
            _ => {}
        }
        let step_mps = AIRSPEED_RATE_MPS2 * STEP_S;
        self.indicated_mps +=
            (self.commanded_ias_mps - self.indicated_mps).clamp(-step_mps, step_mps);
        let velocity = self.ground_velocity();
        let error = match self.demand {
            Some(Demand::Heading(heading)) => {
                heading - (self.heading_rad + self.reference_bias_rad)
            }
            Some(Demand::Course(course)) => {
                course - (velocity[1].atan2(velocity[0]) + self.reference_bias_rad)
            }
            None => 0.0,
        };
        let error = error.sin().atan2(error.cos());
        let rate_per_tangent = GRAVITY_MPS2 / self.airspeed_mps();
        let wanted_bank = self.bank_command(error, rate_per_tangent);
        let roll_rate = ((wanted_bank - self.bank_rad) / ROLL_TIME_CONSTANT_S)
            .clamp(-MAX_ROLL_RATE_RPS, MAX_ROLL_RATE_RPS);
        self.bank_rad += roll_rate * STEP_S;
        self.heading_rad += rate_per_tangent * self.bank_rad.tan() * STEP_S;
        let velocity = self.ground_velocity();
        self.ned[0] += velocity[0] * STEP_S;
        self.ned[1] += velocity[1] * STEP_S;
    }

    /// Aviate's `bank_command`.
    fn bank_command(&mut self, error: f64, rate_per_tangent: f64) -> f64 {
        let limit = ROUTE_BANK_RAD;
        let proportional = error / COURSE_TIME_CONSTANT_S;
        let bank_of = |turn_rate: f64| (turn_rate / rate_per_tangent).atan();
        let unlimited = bank_of(proportional + self.turn_rate_integrator);
        let bank = unlimited.clamp(-limit, limit);
        let near_target = bank_of(proportional).abs() <= 0.5 * limit;
        if near_target && (unlimited - bank) * error <= 0.0 {
            let bound = limit.tan() * rate_per_tangent;
            self.turn_rate_integrator = (self.turn_rate_integrator
                + proportional / COURSE_INTEGRAL_TIME_S * STEP_S)
                .clamp(-bound, bound);
        }
        bank
    }
}

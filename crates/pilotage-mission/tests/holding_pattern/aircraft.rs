//! A fixed wing with Aviate's C172 lateral laws: the proportional
//! heading-to-bank law for heading and altitude demands, as on the onboard
//! navigator, and the trimmed course law for velocity demands, as on
//! simulation truth. It flies in a steady wind.

use navigate_contract::{
    AttitudeQuaternion, ClockDomainId, Ellipsoid, FaultDetection, GeodeticPosition,
    IntegrityAssessment, MonotonicNanos, NavigationSolution, NedVelocity, Redundancy, SensorClass,
    SolutionQuality, SolutionStamp, SourceComposition, SourceEpoch, SymmetricCov3,
    WrappingSequence,
};
use navigate_geodesy::NedOffset;
use navigate_geodesy::wgs84::tangent_plane;
use pilotage_mission::true_airspeed_for;
use pilotage_protocol::ControlIntent;

pub const CLOCK: ClockDomainId = ClockDomainId::new(7);
pub const STEP_NANOS: u64 = 50_000_000;
pub const STEP_S: f64 = 0.05;
pub const ALTITUDE_M: f64 = 2_000.0;
pub const CRUISE_IAS_MPS: f64 = 55.0;
const GRAVITY_MPS2: f64 = 9.806_65;
/// Aviate C172 `route_bank_angle`, `course_time_constant`,
/// `course_integral_time`, `roll_time_constant`, and `max_roll_rate`.
pub const BANK_LIMIT_RAD: f64 = 0.44;
const COURSE_TIME_CONSTANT_S: f64 = 3.0;
const COURSE_INTEGRAL_TIME_S: f64 = 20.0;
const ROLL_TIME_CONSTANT_S: f64 = 0.5;
const MAX_ROLL_RATE_RPS: f64 = 0.5;

pub fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(40.0_f64.to_radians(), -75.0_f64.to_radians(), ALTITUDE_M)
}

/// A fixed wing in the anchor's NED plane.
pub struct Aircraft {
    pub ned: [f64; 3],
    /// The heading, or the course for the velocity law, radians.
    pub heading_rad: f64,
    pub bank_rad: f64,
    pub indicated_mps: f64,
    turn_rate_integrator: f64,
    /// The wind, north and east m/s.
    pub wind_mps: [f64; 2],
}

impl Aircraft {
    pub fn new(ned: [f64; 3], heading_rad: f64, wind_mps: [f64; 2]) -> Self {
        Self {
            ned,
            heading_rad,
            bank_rad: 0.0,
            indicated_mps: CRUISE_IAS_MPS,
            turn_rate_integrator: 0.0,
            wind_mps,
        }
    }

    fn airspeed_mps(&self) -> f64 {
        true_airspeed_for(self.indicated_mps, ALTITUDE_M - self.ned[2])
    }

    pub fn ground_velocity(&self) -> [f64; 3] {
        let airspeed = self.airspeed_mps();
        [
            airspeed * self.heading_rad.cos() + self.wind_mps[0],
            airspeed * self.heading_rad.sin() + self.wind_mps[1],
            0.0,
        ]
    }

    /// Flies one step of `intent`: a heading and altitude demand, or a
    /// velocity demand whose direction is a course and whose magnitude is
    /// an indicated airspeed. No intent holds the last demand.
    pub fn fly(&mut self, intent: Option<&ControlIntent>) {
        let mut wanted = self.heading_rad;
        let mut altitude_m = None;
        let mut heading_law = false;
        match intent {
            Some(ControlIntent::HeadingAltitude(demand)) => {
                heading_law = true;
                wanted = f64::from(demand.heading_rad);
                altitude_m = Some(f64::from(demand.altitude_m));
                if let Some(airspeed) = demand.airspeed_mps {
                    self.indicated_mps = f64::from(airspeed);
                }
            }
            Some(ControlIntent::Velocity(v)) => {
                let (sin, cos) = self.heading_rad.sin_cos();
                let (vx, vy) = (f64::from(v.vx), f64::from(v.vy));
                let [north, east] = [vx * cos - vy * sin, vx * sin + vy * cos];
                if north.hypot(east) > 0.0 {
                    wanted = east.atan2(north);
                    self.indicated_mps = north.hypot(east).clamp(33.0, 65.0);
                }
            }
            _ => {}
        }
        let rate_per_tangent = GRAVITY_MPS2 / self.airspeed_mps();
        let error = (wanted - self.heading_rad)
            .sin()
            .atan2((wanted - self.heading_rad).cos());
        // Aviate holds a heading demand with the proportional heading-to-bank
        // law, and a velocity demand's course with the trimmed course law.
        let wanted_bank = if heading_law {
            (error / COURSE_TIME_CONSTANT_S / rate_per_tangent)
                .atan()
                .clamp(-BANK_LIMIT_RAD, BANK_LIMIT_RAD)
        } else {
            self.bank_command(error, rate_per_tangent)
        };
        let roll_rate = ((wanted_bank - self.bank_rad) / ROLL_TIME_CONSTANT_S)
            .clamp(-MAX_ROLL_RATE_RPS, MAX_ROLL_RATE_RPS);
        self.bank_rad += roll_rate * STEP_S;
        self.heading_rad += rate_per_tangent * self.bank_rad.tan() * STEP_S;
        if let Some(altitude_m) = altitude_m {
            let height_error = altitude_m - (ALTITUDE_M - self.ned[2]);
            self.ned[2] -= (height_error / 5.0).clamp(-5.0, 2.5) * STEP_S;
        }
        let velocity = self.ground_velocity();
        self.ned[0] += velocity[0] * STEP_S;
        self.ned[1] += velocity[1] * STEP_S;
    }

    /// Aviate's `bank_command` for a course error: a turn rate over the
    /// course time constant, with an integrator that trims a sustained
    /// turn near the target, limited to the route bank.
    fn bank_command(&mut self, error: f64, rate_per_tangent: f64) -> f64 {
        let limit = BANK_LIMIT_RAD;
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

    /// The onboard navigator's solution of the aircraft.
    pub fn solution(&self, now: MonotonicNanos, sequence: u32) -> NavigationSolution {
        const SIGMA_M: f64 = 5.0;
        let plane = tangent_plane(anchor()).expect("anchor is plausible");
        let position = plane.from_ned(&NedOffset::new(self.ned[0], self.ned[1], self.ned[2]));
        let velocity = self.ground_velocity();
        let composition = SourceComposition::of(SensorClass::VisualLandmark);
        let variance = SIGMA_M * SIGMA_M;
        let half = self.heading_rad / 2.0;
        NavigationSolution::new(
            SolutionStamp::new(
                SourceEpoch::new(0),
                WrappingSequence::new(sequence),
                now,
                CLOCK,
            ),
            position,
            NedVelocity::new(velocity[0], velocity[1], velocity[2]),
            SymmetricCov3::from_diagonal(variance, variance, variance),
            SymmetricCov3::from_diagonal(0.25, 0.25, 0.25),
            IntegrityAssessment::new(
                SolutionQuality::Good,
                composition,
                Redundancy::None,
                SIGMA_M,
                SIGMA_M,
                FaultDetection::Unavailable,
            ),
            composition,
            Ellipsoid::WGS84,
        )
        .with_attitude(AttitudeQuaternion::new(half.cos(), 0.0, 0.0, half.sin()))
    }
}

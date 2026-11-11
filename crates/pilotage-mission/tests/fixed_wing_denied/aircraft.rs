//! The simulated airframe the denied flights steer: a fixed wing whose
//! flight controller holds the heading, altitude, and airspeed it is
//! commanded.

use pilotage_mission::true_airspeed_for;
use pilotage_protocol::{AltitudeReference, ControlIntent};

use super::{
    ALTITUDE_M, COURSE_TIME_CONSTANT_S, GRAVITY_MPS2, MAX_ROLL_RATE_RPS, ROLL_TIME_CONSTANT_S,
    ROUTE_BANK_RAD, STEP_S,
};

/// A fixed wing in a steady wind holding the heading, altitude, and
/// indicated airspeed it is commanded, with Aviate's heading-to-bank law.
pub(super) struct Aircraft {
    pub(super) ned: [f64; 3],
    pub(super) heading_rad: f64,
    pub(super) bank_rad: f64,
    pub(super) indicated_mps: f64,
    /// The wind, north and east, m/s.
    pub(super) wind: [f64; 2],
    /// How far the heading the flight controller holds is from the true
    /// heading it is commanded, radians: a heading reference that is not
    /// true north (a magnetic one), or an estimator's heading error.
    pub(super) heading_bias_rad: f64,
}

impl Aircraft {
    pub(super) fn ground_velocity(&self) -> [f64; 3] {
        let airspeed = true_airspeed_for(self.indicated_mps, ALTITUDE_M - self.ned[2]);
        [
            airspeed * self.heading_rad.cos() + self.wind[0],
            airspeed * self.heading_rad.sin() + self.wind[1],
            0.0,
        ]
    }

    pub(super) fn fly(&mut self, intent: &ControlIntent) {
        let ControlIntent::HeadingAltitude(demand) = intent else {
            panic!("a fixed wing on the onboard navigator flies heading and altitude: {intent:?}");
        };
        assert_eq!(demand.reference, AltitudeReference::Ellipsoid);
        if let Some(airspeed) = demand.airspeed_mps {
            self.indicated_mps = f64::from(airspeed);
        }
        let airspeed = true_airspeed_for(self.indicated_mps, ALTITUDE_M - self.ned[2]);
        let rate_per_tangent = GRAVITY_MPS2 / airspeed;
        let wanted = f64::from(demand.heading_rad) + self.heading_bias_rad;
        let error = (wanted - self.heading_rad)
            .sin()
            .atan2((wanted - self.heading_rad).cos());
        let wanted_bank = (error / COURSE_TIME_CONSTANT_S / rate_per_tangent)
            .atan()
            .clamp(-ROUTE_BANK_RAD, ROUTE_BANK_RAD);
        let roll_rate = ((wanted_bank - self.bank_rad) / ROLL_TIME_CONSTANT_S)
            .clamp(-MAX_ROLL_RATE_RPS, MAX_ROLL_RATE_RPS);
        self.bank_rad += roll_rate * STEP_S;
        self.heading_rad += rate_per_tangent * self.bank_rad.tan() * STEP_S;
        // The altitude loop: a climb or sink rate toward the demand.
        let height_error = f64::from(demand.altitude_m) - (ALTITUDE_M - self.ned[2]);
        self.ned[2] -= (height_error / 5.0).clamp(-5.0, 2.5) * STEP_S;
    }

    pub(super) fn step(&mut self) {
        let velocity = self.ground_velocity();
        self.ned[0] += velocity[0] * STEP_S;
        self.ned[1] += velocity[1] * STEP_S;
    }
}

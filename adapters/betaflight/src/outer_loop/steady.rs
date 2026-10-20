//! The hover throttle as steady vertical flight reveals it.
//!
//! In steady vertical flight, climbing or not, the level-flight throttle
//! balances the weight: what it takes against drag at the climb rates an
//! operator flies is a few percent, so the estimate errs high, toward a
//! slow climb. It stands in for the hover throttle before the quad has
//! hovered settled, where the climb-rate integrator would otherwise: that
//! integrator carries the braking of the moment, so right after a climb
//! stops it reads several percent under hover, and a direct flight
//! entered then would hold it and sink.

/// Below this vertical acceleration, m/s², the climb rate is steady.
const STEADY_ACCEL: f64 = 0.3;
/// Time constant of the vertical-acceleration filter, s.
const ACCEL_TIME_CONSTANT_S: f64 = 0.3;
/// How long the climb rate must stay steady before it teaches, s; the
/// filter lags, so the first moments of a change still read steady.
const STEADY_DWELL_S: f64 = 0.5;
/// How fast the estimate follows the level-flight throttle, 1/s.
const LEARN_RATE: f64 = 1.0;

/// What steady vertical flight has taught about the hover throttle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct SteadyHover {
    estimate: Option<f64>,
    last_climb: Option<f64>,
    accel: f64,
    steady_s: f64,
}

impl SteadyHover {
    /// The hover throttle steady flight has taught, if it has.
    pub(crate) fn estimate(&self) -> Option<f64> {
        self.estimate
    }

    /// Forgets the climb-rate history but keeps what was learned: the
    /// next sample starts a fresh acceleration estimate.
    pub(crate) fn restart(&mut self) {
        *self = Self {
            estimate: self.estimate,
            ..Self::default()
        };
    }

    /// One airborne step: `level` is the commanded throttle with the tilt
    /// taken out, `climb_rate` the measured climb, m/s. `trusted` is false
    /// when the sample cannot teach (on the ground, throttle saturated).
    pub(crate) fn observe(&mut self, level: f64, climb_rate: f64, trusted: bool, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        if let Some(last) = self.last_climb {
            let raw = (climb_rate - last) / dt;
            self.accel += (raw - self.accel) * (dt / ACCEL_TIME_CONSTANT_S).min(1.0);
        }
        self.last_climb = Some(climb_rate);
        let steady = trusted && self.accel.abs() < STEADY_ACCEL;
        self.steady_s = if steady { self.steady_s + dt } else { 0.0 };
        if self.steady_s < STEADY_DWELL_S {
            return;
        }
        self.estimate = Some(match self.estimate {
            Some(hover) => hover + (level - hover) * (LEARN_RATE * dt).min(1.0),
            None => level,
        });
    }
}

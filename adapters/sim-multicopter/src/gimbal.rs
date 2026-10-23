//! The three-axis gimbal: pitch under rate control, roll and pitch held
//! to the horizon, yaw following the body.

/// The lowest pitch, radians (straight down).
pub const PITCH_MIN_RAD: f64 = -core::f64::consts::FRAC_PI_2;
/// The highest pitch, radians (60 degrees up).
pub const PITCH_MAX_RAD: f64 = core::f64::consts::FRAC_PI_3;
/// The pitch-rate limit, radians per second. It is the host policy of the
/// DJI adapter.
pub const PITCH_RATE_LIMIT_RPS: f32 = core::f32::consts::PI / 6.0;

/// The gimbal pitch state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gimbal {
    pitch_rad: f64,
    rate_rps: f64,
    recentering: bool,
}

impl Gimbal {
    /// A gimbal at `pitch_rad`, clamped to the mechanical range.
    #[must_use]
    pub fn at(pitch_rad: f64) -> Self {
        Self {
            pitch_rad: clamp_pitch(pitch_rad),
            rate_rps: 0.0,
            recentering: false,
        }
    }

    /// The present pitch, radians.
    #[must_use]
    pub fn pitch_rad(&self) -> f64 {
        self.pitch_rad
    }

    /// Sets the commanded pitch rate. A rate command ends a recenter.
    /// Returns true when the rate was limited.
    pub fn command_rate(&mut self, rate_rps: f64) -> bool {
        let limit = f64::from(PITCH_RATE_LIMIT_RPS);
        let rate = if rate_rps.is_nan() {
            0.0
        } else {
            rate_rps.clamp(-limit, limit)
        };
        self.rate_rps = rate;
        self.recentering = false;
        rate != rate_rps
    }

    /// Stops the pitch motion.
    pub fn stop(&mut self) {
        self.rate_rps = 0.0;
    }

    /// Starts a slew to the horizon at the rate limit.
    pub fn recenter(&mut self) {
        self.recentering = true;
        self.rate_rps = 0.0;
    }

    /// Advances the pitch by `dt_s`.
    pub fn step(&mut self, dt_s: f64) {
        if self.recentering {
            let step = f64::from(PITCH_RATE_LIMIT_RPS) * dt_s;
            if self.pitch_rad.abs() <= step {
                self.pitch_rad = 0.0;
                self.recentering = false;
            } else {
                self.pitch_rad -= step * self.pitch_rad.signum();
            }
            return;
        }
        self.pitch_rad = clamp_pitch(self.pitch_rad + self.rate_rps * dt_s);
    }
}

fn clamp_pitch(pitch_rad: f64) -> f64 {
    if pitch_rad.is_nan() {
        0.0
    } else {
        pitch_rad.clamp(PITCH_MIN_RAD, PITCH_MAX_RAD)
    }
}

#[cfg(test)]
mod tests;

//! A three-axis camera gimbal driven by PX4's gimbal outputs.
//!
//! PX4's gimbal module, in its AUX output mode, maps each angle setpoint
//! onto one output in [-1, 1]: 0 is level, 1 the axis's upper limit, -1
//! its lower one. The model maps the outputs back with the same limits
//! and follows them with a servo's lag. PX4 sends no outputs while
//! disarmed, so the gimbal then holds where it was.

/// One axis's travel, degrees: `min` below level, `max` above.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisLimits {
    /// The angle at output -1.
    pub min_deg: f64,
    /// The angle at output 1.
    pub max_deg: f64,
}

/// The gimbal's travel on each axis, as PX4's `MNT_RANGE_ROLL`,
/// `MNT_MIN_PITCH`/`MNT_MAX_PITCH`, and `MNT_RANGE_YAW` state it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GimbalLimits {
    /// Roll.
    pub roll: AxisLimits,
    /// Pitch.
    pub pitch: AxisLimits,
    /// Yaw.
    pub yaw: AxisLimits,
}

impl GimbalLimits {
    /// The limits PX4's parameters name: the full roll and yaw ranges,
    /// centred, and the pitch bounds.
    pub fn from_px4(
        range_roll_deg: f64,
        max_pitch_deg: f64,
        min_pitch_deg: f64,
        range_yaw_deg: f64,
    ) -> Self {
        Self {
            roll: AxisLimits {
                min_deg: -range_roll_deg / 2.0,
                max_deg: range_roll_deg / 2.0,
            },
            pitch: AxisLimits {
                min_deg: min_pitch_deg,
                max_deg: max_pitch_deg,
            },
            yaw: AxisLimits {
                min_deg: -range_yaw_deg / 2.0,
                max_deg: range_yaw_deg / 2.0,
            },
        }
    }
}

/// The outputs (0-based) PX4 drives the gimbal on: `PWM_MAIN_FUNC5..7`.
pub const GIMBAL_OUTPUTS: [usize; 3] = [4, 5, 6];
/// The servo time constant, s.
const SERVO_TAU_S: f64 = 0.05;

impl AxisLimits {
    fn angle_deg(self, output: f64) -> f64 {
        let output = if output.is_finite() {
            output.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        if output >= 0.0 {
            output * self.max_deg
        } else {
            -output * self.min_deg
        }
    }
}

/// The gimbal's angles relative to the vehicle body, degrees: roll,
/// pitch (positive up), yaw (positive right).
#[derive(Debug, Clone, PartialEq)]
pub struct Gimbal {
    limits: GimbalLimits,
    angles_deg: [f64; 3],
}

impl Gimbal {
    /// A level gimbal.
    pub fn new(limits: GimbalLimits) -> Self {
        Self {
            limits,
            angles_deg: [0.0; 3],
        }
    }

    /// The current angles.
    pub fn angles_deg(&self) -> [f64; 3] {
        self.angles_deg
    }

    /// Follows the outputs for `dt_s`; `None` (disarmed, no outputs) holds.
    pub fn step(&mut self, outputs: Option<[f64; 3]>, dt_s: f64) {
        let Some(outputs) = outputs else {
            return;
        };
        let axes = [self.limits.roll, self.limits.pitch, self.limits.yaw];
        let blend = 1.0 - (-dt_s / SERVO_TAU_S).exp();
        for ((angle, axis), output) in self.angles_deg.iter_mut().zip(axes).zip(outputs) {
            let target = axis.angle_deg(output);
            *angle += (target - *angle) * blend;
        }
    }
}

#[cfg(test)]
mod tests;

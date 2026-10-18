//! The RC frame the adapter sends as `MSP_SET_RAW_RC`.
//!
//! Channel order is Betaflight's default `AETR1234` map: roll, pitch,
//! throttle, yaw, then AUX1 (arm) and AUX2 (ANGLE when high, ACRO when
//! low). The configuration in `sim/betaflight/sitl.cli` (and on a real
//! board, the same settings) makes the stick maps linear: in ANGLE, roll
//! and pitch deflection times [`ANGLE_LIMIT_DEG`] is the commanded tilt;
//! on every axis in ACRO, and on yaw in ANGLE, deflection times
//! [`MAX_RATE_DPS`] is the commanded rate.

use pilotage_msp::ModeRange;
use pilotage_msp::message::permanent_id;

/// Centre pulse, µs.
pub(crate) const CENTER_US: u16 = 1_500;
/// Lowest pulse, µs: throttle cut and switches off.
pub(crate) const LOW_US: u16 = 1_000;
/// Half the stick travel, µs.
const HALF_TRAVEL_US: f64 = 500.0;
/// AUX1 pulse that activates ARM.
pub(crate) const ARM_HIGH_US: u16 = 1_800;
/// Tilt at full roll or pitch deflection in ANGLE mode (`angle_limit`).
pub(crate) const ANGLE_LIMIT_DEG: f64 = 30.0;
/// Rate at full deflection on every axis (`*_rc_rate` = `*_srate` = 67).
pub(crate) const MAX_RATE_DPS: f64 = 670.0;
/// AUX2 pulse that selects ANGLE; [`LOW_US`] leaves ACRO.
pub(crate) const ANGLE_HIGH_US: u16 = 1_800;
/// Channels in a frame.
pub(crate) const CHANNELS: usize = 8;

/// The aux ranges the adapter's channel layout needs, in slot order.
pub(crate) fn required_mode_ranges() -> [ModeRange; 2] {
    [
        ModeRange::from_pulse(permanent_id::ARM, 0, 1_700, 2_100),
        ModeRange::from_pulse(permanent_id::ANGLE, 1, 1_700, 2_100),
    ]
}

/// Normalized stick positions plus the arm switch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sticks {
    /// Roll, -1..1, positive right.
    pub(crate) roll: f64,
    /// Pitch, -1..1, positive nose down (stick forward).
    pub(crate) pitch: f64,
    /// Throttle, 0..1.
    pub(crate) throttle: f64,
    /// Yaw, -1..1, positive clockwise.
    pub(crate) yaw: f64,
    /// Whether AUX1 asks Betaflight to arm.
    pub(crate) arm: bool,
    /// Whether AUX2 selects ANGLE (self-levelling) rather than ACRO.
    pub(crate) angle: bool,
}

impl Sticks {
    /// Sticks centred, throttle cut, switch off, ANGLE.
    pub(crate) const IDLE: Self = Self {
        roll: 0.0,
        pitch: 0.0,
        throttle: 0.0,
        yaw: 0.0,
        arm: false,
        angle: true,
    };

    /// The channel pulses, µs. Out-of-range or non-finite inputs clamp.
    pub(crate) fn channels(&self) -> [u16; CHANNELS] {
        let bipolar = |v: f64| pulse(CENTER_US, v.clamp(-1.0, 1.0) * HALF_TRAVEL_US);
        let throttle = pulse(LOW_US, self.throttle.clamp(0.0, 1.0) * 2.0 * HALF_TRAVEL_US);
        [
            bipolar(self.roll),
            bipolar(self.pitch),
            throttle,
            bipolar(self.yaw),
            if self.arm { ARM_HIGH_US } else { LOW_US },
            if self.angle { ANGLE_HIGH_US } else { LOW_US },
            CENTER_US,
            CENTER_US,
        ]
    }
}

fn pulse(base: u16, offset_us: f64) -> u16 {
    let offset = if offset_us.is_finite() {
        offset_us.round()
    } else {
        0.0
    };
    // The offset is bounded to ±1000 µs by the callers' clamps.
    (f64::from(base) + offset) as u16
}

#[cfg(test)]
mod tests;

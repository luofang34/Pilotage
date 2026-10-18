//! The fixed-wing velocity envelope and its mapping onto PX4 Position-mode
//! sticks (`src/modules/fw_mode_manager/FixedWingModeManager.cpp`,
//! `control_manual_position`, with `FW_POS_STK_CONF = 2`):
//!
//! - throttle stick, zero-centered, sets the airspeed setpoint piecewise:
//!   -1 → `FW_AIRSPD_MIN`, 0 → `FW_AIRSPD_TRIM`, +1 → `FW_AIRSPD_MAX`;
//! - pitch stick sets the height rate through a 0.06 deadband, + = sink up
//!   to `FW_T_SINK_R_SP`, - = climb up to `FW_T_CLMB_R_SP`;
//! - roll stick sets the bank angle, `stick * FW_R_LIM`; a roll and yaw
//!   stick inside 0.01 engages course hold.
//!
//! The unified velocity intent maps onto that: `vx` is the airspeed change
//! from trim, `vz` the descent rate, and `yaw_rate` the coordinated-turn
//! rate at trim airspeed. A fixed wing cannot move sideways, so `vy` is
//! not flown. Neutral input therefore holds trim airspeed, altitude, and
//! course. The adapter pins every parameter named here (see
//! [`super::params`]), so these constants are the FC's own envelope.

use pilotage_mavlink::codec::ManualControlSticks;
use pilotage_protocol::VelocityIntent;

/// `FW_AIRSPD_MIN`, m/s.
pub(crate) const AIRSPEED_MIN_MPS: f32 = 10.0;
/// `FW_AIRSPD_TRIM`, m/s: the airspeed neutral input holds.
pub(crate) const AIRSPEED_TRIM_MPS: f32 = 15.0;
/// `FW_AIRSPD_MAX`, m/s.
pub(crate) const AIRSPEED_MAX_MPS: f32 = 20.0;
/// `FW_R_LIM`, degrees: the bank angle at full roll stick.
pub(crate) const ROLL_LIMIT_DEG: f32 = 50.0;
/// `FW_T_CLMB_R_SP`, m/s: the climb rate at full back pitch stick.
pub(crate) const CLIMB_RATE_MPS: f32 = 3.0;
/// `FW_T_SINK_R_SP`, m/s: the sink rate at full forward pitch stick.
pub(crate) const SINK_RATE_MPS: f32 = 2.0;

/// The advertised forward bound: the airspeed change from trim. Trim sits
/// midway between the pinned minimum and maximum, so one bound states both.
pub(crate) const MAX_AIRSPEED_CHANGE_MPS: f32 = AIRSPEED_MAX_MPS - AIRSPEED_TRIM_MPS;
/// The advertised vertical bound: the smaller of climb and sink, so a
/// full-envelope demand is reachable in both directions.
pub(crate) const MAX_VERTICAL_MPS: f32 = if CLIMB_RATE_MPS < SINK_RATE_MPS {
    CLIMB_RATE_MPS
} else {
    SINK_RATE_MPS
};

/// PX4's height-rate stick deadband (`kStickDeadBand`).
const PITCH_DEADBAND: f32 = 0.06;
const GRAVITY_MPS2: f32 = 9.806_65;
/// Neutral throttle: 500 on the `[0, 1000]` wire scale is zero-centered 0.
const THROTTLE_CENTER: f32 = 500.0;
const STICK_FULL: f32 = 1000.0;

/// The advertised turn-rate bound: a coordinated turn at full bank and
/// trim airspeed, `g * tan(FW_R_LIM) / V_trim`.
#[must_use]
pub(crate) fn max_turn_rate_rps() -> f32 {
    GRAVITY_MPS2 * ROLL_LIMIT_DEG.to_radians().tan() / AIRSPEED_TRIM_MPS
}

/// Centered sticks: trim airspeed, level flight, course hold.
pub(crate) const NEUTRAL: ManualControlSticks = ManualControlSticks {
    x: 0,
    y: 0,
    z: 500,
    r: 0,
};

/// Converts one velocity intent to Position-mode sticks. The flag reports
/// that the demand was constrained: a component outside the advertised
/// envelope was clamped, or a lateral `vy` was dropped.
#[must_use]
pub(crate) fn sticks_from_velocity(velocity: &VelocityIntent) -> (ManualControlSticks, bool) {
    let (airspeed, airspeed_clamped) = unit(velocity.vx / MAX_AIRSPEED_CHANGE_MPS);
    let (vertical, vertical_clamped) = unit(velocity.vz / MAX_VERTICAL_MPS);
    let (turn, turn_clamped) = unit(velocity.yaw_rate / max_turn_rate_rps());
    let descent_mps = vertical * MAX_VERTICAL_MPS;
    let height_rate_fraction = if descent_mps >= 0.0 {
        descent_mps / SINK_RATE_MPS
    } else {
        descent_mps / CLIMB_RATE_MPS
    };
    let bank_rad = (turn * max_turn_rate_rps() * AIRSPEED_TRIM_MPS / GRAVITY_MPS2).atan();
    let sticks = ManualControlSticks {
        x: wire(through_deadband(height_rate_fraction)),
        y: wire(bank_rad / ROLL_LIMIT_DEG.to_radians()),
        z: (THROTTLE_CENTER + airspeed * THROTTLE_CENTER).round() as i16,
        r: 0,
    };
    let lateral_dropped = velocity.vy != 0.0;
    (
        sticks,
        airspeed_clamped || vertical_clamped || turn_clamped || lateral_dropped,
    )
}

/// Clamps to `[-1, 1]`, treating NaN as neutral; reports whether it clamped.
fn unit(value: f32) -> (f32, bool) {
    if value.is_nan() {
        return (0.0, true);
    }
    let clamped = value.clamp(-1.0, 1.0);
    (clamped, clamped != value)
}

/// The stick deflection that PX4's deadband maps back to `fraction`, so a
/// small demand is flown instead of vanishing inside the deadband.
fn through_deadband(fraction: f32) -> f32 {
    if fraction == 0.0 {
        return 0.0;
    }
    fraction.signum() * (PITCH_DEADBAND + fraction.abs() * (1.0 - PITCH_DEADBAND))
}

fn wire(stick: f32) -> i16 {
    (stick.clamp(-1.0, 1.0) * STICK_FULL).round() as i16
}

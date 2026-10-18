#![allow(clippy::expect_used, clippy::panic)]

use super::{ARM_HIGH_US, Sticks, required_mode_ranges};

#[test]
fn idle_cuts_throttle_and_leaves_the_switch_off() {
    assert_eq!(
        Sticks::IDLE.channels(),
        [1_500, 1_500, 1_000, 1_500, 1_000, 1_800, 1_500, 1_500]
    );
}

#[test]
fn deflections_map_linearly_and_clamp() {
    let sticks = Sticks {
        roll: 0.5,
        pitch: -2.0,
        throttle: 0.55,
        yaw: f64::NAN,
        arm: true,
        angle: false,
    };
    assert_eq!(
        sticks.channels()[..6],
        [1_750, 1_000, 1_550, 1_500, ARM_HIGH_US, 1_000]
    );
}

#[test]
fn the_arm_pulse_falls_in_the_arm_range_and_idle_does_not() {
    let [arm, angle] = required_mode_ranges();
    assert!(arm.contains(ARM_HIGH_US) && !arm.contains(1_000));
    assert!(
        angle.contains(1_800) && !angle.contains(1_000),
        "AUX2 low is ACRO"
    );
}

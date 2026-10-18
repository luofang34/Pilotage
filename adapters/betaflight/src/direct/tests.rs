#![allow(clippy::expect_used, clippy::panic)]

use super::{DirectDemand, DirectLoop};
use crate::rc::{ANGLE_LIMIT_DEG, MAX_RATE_DPS};

fn rates(roll: f64, pitch: f64, yaw: f64, thrust: f64) -> DirectDemand {
    DirectDemand::Rates {
        roll,
        pitch,
        yaw,
        thrust,
    }
}

#[test]
fn body_rates_fly_acro_through_the_linear_rate_curve() {
    let mut direct = DirectLoop::default();
    let max = MAX_RATE_DPS.to_radians();
    let sticks = direct.sticks(rates(0.5 * max, 0.25 * max, -0.1 * max, 0.4), 0.0);
    assert!(!sticks.angle, "ACRO");
    assert!((sticks.roll - 0.5).abs() < 1e-9);
    assert!((sticks.pitch + 0.25).abs() < 1e-9, "nose up is stick back");
    assert!((sticks.yaw + 0.1).abs() < 1e-9);
    assert_eq!(sticks.throttle, 0.4);
    let clamped = direct.sticks(rates(10.0 * max, 0.0, 0.0, 2.0), 0.0);
    assert_eq!((clamped.roll, clamped.throttle), (1.0, 1.0));
}

#[test]
fn an_attitude_flies_angle_and_holds_the_heading_by_rate() {
    let mut direct = DirectLoop::default();
    let sticks = direct.sticks(
        DirectDemand::Attitude {
            roll: 15f64.to_radians(),
            pitch: -6f64.to_radians(),
            heading: 0.1,
            thrust: 0.5,
        },
        -0.1,
    );
    assert!(sticks.angle);
    assert!((sticks.roll - 15.0 / ANGLE_LIMIT_DEG).abs() < 1e-9);
    assert!(
        (sticks.pitch - 6.0 / ANGLE_LIMIT_DEG).abs() < 1e-9,
        "nose down is stick forward"
    );
    assert!(sticks.yaw > 0.0, "turns clockwise toward the heading");
    let across_north = direct.sticks(
        DirectDemand::Attitude {
            roll: 0.0,
            pitch: 0.0,
            heading: 0.05,
            thrust: 0.5,
        },
        std::f64::consts::TAU - 0.05,
    );
    assert!(
        across_north.yaw > 0.0 && across_north.yaw < 0.05,
        "the short way round"
    );
}

#[test]
fn entering_direct_holds_the_throttle_until_the_operator_reaches_it() {
    let mut direct = DirectLoop::default();
    direct.begin(0.57);
    assert_eq!(direct.sticks(rates(0.0, 0.0, 0.0, 0.0), 0.0).throttle, 0.57);
    assert_eq!(direct.sticks(rates(0.0, 0.0, 0.0, 0.3), 0.0).throttle, 0.57);
    assert_eq!(
        direct.sticks(rates(0.0, 0.0, 0.0, 0.55), 0.0).throttle,
        0.55
    );
    assert_eq!(
        direct.sticks(rates(0.0, 0.0, 0.0, 0.2), 0.0).throttle,
        0.2,
        "the operator has it now"
    );
}

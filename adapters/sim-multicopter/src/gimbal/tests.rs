use super::{Gimbal, PITCH_MAX_RAD, PITCH_MIN_RAD, PITCH_RATE_LIMIT_RPS};

#[test]
fn pitch_stays_inside_the_mechanical_range() {
    let mut gimbal = Gimbal::at(0.0);
    gimbal.command_rate(-0.5);
    for _ in 0..1_000 {
        gimbal.step(0.01);
    }
    assert_eq!(gimbal.pitch_rad(), PITCH_MIN_RAD);
    gimbal.command_rate(0.5);
    for _ in 0..1_000 {
        gimbal.step(0.01);
    }
    assert_eq!(gimbal.pitch_rad(), PITCH_MAX_RAD);
}

#[test]
fn a_fast_rate_is_limited() {
    let mut gimbal = Gimbal::at(0.0);
    assert!(gimbal.command_rate(10.0));
    gimbal.step(1.0);
    assert!((gimbal.pitch_rad() - f64::from(PITCH_RATE_LIMIT_RPS)).abs() < 1e-6);
}

#[test]
fn recenter_returns_to_the_horizon() {
    let mut gimbal = Gimbal::at(-1.2);
    gimbal.recenter();
    for _ in 0..300 {
        gimbal.step(0.01);
    }
    assert_eq!(gimbal.pitch_rad(), 0.0);
}

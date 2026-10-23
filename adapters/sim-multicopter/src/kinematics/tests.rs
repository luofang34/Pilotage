use super::{BodyVelocity, Kinematics, wrap_pi};
use crate::config::Limits;

const DT: f64 = 0.01;

fn airborne() -> Kinematics {
    let mut state = Kinematics::at_rest(0.0);
    state.pos_ned_m[2] = -50.0;
    state
}

fn run(state: &mut Kinematics, command: BodyVelocity, seconds: f64) {
    let limits = Limits::default();
    for _ in 0..(seconds / DT).round() as u32 {
        state.step(command, &limits, DT);
    }
}

#[test]
fn velocity_reaches_63_percent_after_one_time_constant() {
    let mut state = airborne();
    let command = BodyVelocity {
        forward: 10.0,
        ..BodyVelocity::default()
    };
    run(&mut state, command, 0.5);
    let fraction = state.vel_ned_mps[0] / 10.0;
    assert!((fraction - 0.632).abs() < 0.01, "{fraction}");
    run(&mut state, command, 4.5);
    assert!((state.vel_ned_mps[0] - 10.0).abs() < 1e-3);
}

#[test]
fn body_forward_follows_the_heading() {
    let mut state = airborne();
    state.yaw_rad = 90.0_f64.to_radians();
    run(
        &mut state,
        BodyVelocity {
            forward: 5.0,
            ..BodyVelocity::default()
        },
        5.0,
    );
    assert!(state.vel_ned_mps[0].abs() < 1e-6, "{:?}", state.vel_ned_mps);
    assert!((state.vel_ned_mps[1] - 5.0).abs() < 0.01);
}

#[test]
fn commands_are_limited_to_the_envelope() {
    let limits = Limits::default();
    let (clamped, changed) = BodyVelocity {
        forward: 30.0,
        right: 0.0,
        down: -10.0,
        yaw_rate: 10.0,
    }
    .clamped(&limits);
    assert!(changed);
    assert!((clamped.forward - 15.0).abs() < 1e-9);
    assert!((clamped.down + 4.0).abs() < 1e-9);
    assert!((clamped.yaw_rate - 100.0_f64.to_radians()).abs() < 1e-9);
    let mut state = airborne();
    run(&mut state, clamped, 10.0);
    assert!(state.ground_speed_mps() <= 15.0 + 1e-9);
    assert!(state.vel_ned_mps[2] >= -4.0 - 1e-9);
}

#[test]
fn a_diagonal_command_keeps_its_direction_at_the_limit() {
    let limits = Limits::default();
    let (clamped, _) = BodyVelocity {
        forward: 20.0,
        right: 20.0,
        ..BodyVelocity::default()
    }
    .clamped(&limits);
    assert!((clamped.forward.hypot(clamped.right) - 15.0).abs() < 1e-9);
    assert!((clamped.forward - clamped.right).abs() < 1e-9);
}

#[test]
fn nan_commands_become_zero() {
    let (clamped, changed) = BodyVelocity {
        forward: f64::NAN,
        ..BodyVelocity::default()
    }
    .clamped(&Limits::default());
    assert!(changed);
    assert_eq!(clamped, BodyVelocity::default());
}

#[test]
fn the_ground_stops_a_descent() {
    let mut state = Kinematics::at_rest(0.0);
    state.pos_ned_m[2] = -1.0;
    let limits = Limits::default();
    let command = BodyVelocity {
        down: 2.0,
        ..BodyVelocity::default()
    };
    let mut touched = false;
    for _ in 0..500 {
        touched |= state.step(command, &limits, DT);
    }
    assert!(touched);
    assert_eq!(state.pos_ned_m[2], 0.0);
    assert_eq!(state.vel_ned_mps[2], 0.0);
}

#[test]
fn yaw_rate_turns_and_wraps() {
    let mut state = airborne();
    run(
        &mut state,
        BodyVelocity {
            yaw_rate: 1.0,
            ..BodyVelocity::default()
        },
        10.0,
    );
    assert!(state.yaw_rad > -core::f64::consts::PI && state.yaw_rad <= core::f64::consts::PI);
    assert!((state.yaw_rate_rps - 1.0).abs() < 1e-6);
    assert!((wrap_pi(3.0 * core::f64::consts::PI) - core::f64::consts::PI).abs() < 1e-12);
}

#[test]
fn forward_acceleration_pitches_the_nose_down() {
    let mut state = airborne();
    let limits = Limits::default();
    state.step(
        BodyVelocity {
            forward: 10.0,
            ..BodyVelocity::default()
        },
        &limits,
        DT,
    );
    let (roll, pitch) = state.tilt_rad();
    assert!(pitch < 0.0, "{pitch}");
    assert!(roll.abs() < 1e-9);
}

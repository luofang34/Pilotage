#![allow(clippy::expect_used, clippy::panic)]

use super::{QuadParams, QuadState};
use crate::rotation::{Quat, Vec3};

const DT: f64 = 0.001;
/// Not standard gravity, so a model that ignores its parameter fails.
const GRAVITY: f64 = 9.79;

fn run(state: &mut QuadState, params: &QuadParams, commands: [f64; 4], seconds: f64) {
    for _ in 0..(seconds / DT).round() as u32 {
        state.step(params, commands, DT);
    }
}

fn airborne(motors: f64) -> QuadState {
    QuadState {
        position: Vec3::new(0.0, 0.0, -50.0),
        motors: [motors; 4],
        on_ground: false,
        ..QuadState::landed(&QuadParams::six_inch(GRAVITY), Vec3::ZERO, 0.0)
    }
}

#[test]
fn hover_command_holds_altitude_and_reads_one_g() {
    let params = QuadParams::six_inch(GRAVITY);
    let hover = params.hover_command();
    assert!((0.5..0.65).contains(&hover), "hover at {hover}");
    let mut state = airborne(hover);
    run(&mut state, &params, [hover; 4], 3.0);
    assert!(
        (state.position.z + 50.0).abs() < 1e-3,
        "drifted to {:?}",
        state.position
    );
    assert!(state.rates.norm() < 1e-9);
    let expected = Vec3::new(0.0, 0.0, -GRAVITY);
    assert!((state.specific_force - expected).norm() < 1e-6);
}

#[test]
fn free_fall_follows_gravity_and_reads_weightless() {
    let params = QuadParams {
        linear_drag: 0.0,
        quadratic_drag: 0.0,
        ..QuadParams::six_inch(GRAVITY)
    };
    let mut state = airborne(0.0);
    run(&mut state, &params, [0.0; 4], 1.0);
    let fallen = state.position.z + 50.0;
    assert!((fallen - 0.5 * GRAVITY).abs() < 0.01, "fell {fallen} m");
    assert!((state.velocity.z - GRAVITY).abs() < 1e-6);
    assert!(state.specific_force.norm() < 1e-6, "weightless");
}

#[test]
fn drag_bounds_the_fall_speed() {
    let params = QuadParams::six_inch(GRAVITY);
    let mut state = airborne(0.0);
    state.position.z = -5_000.0;
    run(&mut state, &params, [0.0; 4], 30.0);
    let weight = params.mass * GRAVITY;
    let v = state.velocity.z;
    let drag = params.linear_drag * v + params.quadratic_drag * v * v;
    assert!((drag - weight).abs() < 0.01, "terminal speed {v}");
}

#[test]
fn the_ground_holds_a_quad_below_hover_thrust() {
    let params = QuadParams::six_inch(GRAVITY);
    let mut state = QuadState::landed(&params, Vec3::new(3.0, 4.0, 0.0), 1.0);
    run(&mut state, &params, [0.3; 4], 2.0);
    assert!(state.on_ground);
    assert_eq!(state.position, Vec3::new(3.0, 4.0, 0.0));
    assert_eq!(state.velocity, Vec3::ZERO);
    assert_eq!(state.rates, Vec3::ZERO);
    let (roll, pitch, yaw) = state.attitude.to_euler();
    assert!(roll.abs() < 1e-12 && pitch.abs() < 1e-12);
    assert!((yaw - 1.0).abs() < 1e-9, "heading kept");
    assert!((state.specific_force - Vec3::new(0.0, 0.0, -GRAVITY)).norm() < 1e-9);
}

#[test]
fn thrust_above_weight_lifts_off_and_landing_stops_on_the_ground() {
    let params = QuadParams::six_inch(GRAVITY);
    let mut state = QuadState::landed(&params, Vec3::ZERO, 0.0);
    run(&mut state, &params, [0.7; 4], 1.0);
    assert!(!state.on_ground);
    assert!(state.position.z < -0.5, "climbed to {}", -state.position.z);
    run(&mut state, &params, [0.0; 4], 3.0);
    assert!(state.on_ground);
    assert_eq!(state.position.z, 0.0);
    assert_eq!(state.velocity, Vec3::ZERO);
}

#[test]
fn ground_friction_stops_a_sliding_quad() {
    let params = QuadParams::six_inch(GRAVITY);
    let mut state = QuadState::landed(&params, Vec3::ZERO, 0.0);
    state.velocity = Vec3::new(2.0, 0.0, 0.0);
    state.on_ground = false;
    run(&mut state, &params, [0.0; 4], 1.0);
    assert_eq!(state.velocity, Vec3::ZERO);
}

#[test]
fn motor_differences_torque_the_body_like_the_betaflight_mixer() {
    let params = QuadParams::six_inch(GRAVITY);
    let hover = params.hover_command();
    let d = 0.05;
    // Mixer order: rear right, front right, rear left, front left.
    let cases = [
        (
            [hover - d, hover - d, hover + d, hover + d],
            Vec3::new(1.0, 0.0, 0.0),
        ),
        (
            [hover - d, hover + d, hover - d, hover + d],
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (
            [hover - d, hover + d, hover + d, hover - d],
            Vec3::new(0.0, 0.0, 1.0),
        ),
    ];
    for (commands, axis) in cases {
        let mut state = airborne(hover);
        run(&mut state, &params, commands, 0.05);
        let along = state.rates.dot(axis);
        let across = (state.rates - axis * along).norm();
        assert!(along > 0.1, "{commands:?} turned {:?}", state.rates);
        assert!(
            across < along * 0.01,
            "{commands:?} coupled into {:?}",
            state.rates
        );
    }
}

#[test]
fn a_nose_down_tilt_accelerates_forward() {
    let params = QuadParams::six_inch(GRAVITY);
    let hover = params.hover_command();
    let mut state = airborne(hover);
    state.attitude = Quat::from_euler(0.0, (-10.0f64).to_radians(), 90f64.to_radians());
    run(&mut state, &params, [hover; 4], 0.5);
    assert!(
        state.velocity.y > 0.5,
        "east (the heading) at {:?}",
        state.velocity
    );
    assert!(state.velocity.x.abs() < 0.01);
}

#[test]
fn motor_lag_is_first_order() {
    let params = QuadParams::six_inch(GRAVITY);
    let mut state = QuadState::landed(&params, Vec3::ZERO, 0.0);
    run(&mut state, &params, [1.0; 4], params.motor_time_constant);
    let reached = state.motors[0];
    assert!(
        (reached - (1.0 - (-1.0f64).exp())).abs() < 1e-3,
        "{reached}"
    );
}

#[test]
fn non_finite_and_out_of_range_commands_are_contained() {
    let params = QuadParams::six_inch(GRAVITY);
    let mut state = QuadState::landed(&params, Vec3::ZERO, 0.0);
    run(
        &mut state,
        &params,
        [f64::NAN, f64::INFINITY, -3.0, 0.0],
        0.5,
    );
    assert!(state.motors.iter().all(|m| (0.0..=1.0).contains(m)));
    assert_eq!(state.motors[0], 0.0);
    assert!(state.on_ground);
}

//! The motor model: idle, full, lag, and yaw torque balance.

use super::{QuadMotors, X500_MOTOR};

fn settle(motors: &mut QuadMotors, commands: [f64; 4], armed: bool) -> super::RotorOutput {
    let mut out = motors.step(commands, armed, 0.004);
    for _ in 0..500 {
        out = motors.step(commands, armed, 0.004);
    }
    out
}

#[test]
fn a_full_command_gives_the_x500s_maximum_thrust() {
    let mut motors = QuadMotors::new(X500_MOTOR);
    let out = settle(&mut motors, [1.0; 4], true);
    for thrust in out.thrust_n {
        assert!((thrust - 8.548_58).abs() < 1e-6, "{thrust}");
    }
    // Equal thrust on a balanced quad X cancels the yaw torque.
    assert!(out.yaw_torque_nm.abs() < 1e-9);
    // The x500 hovers 2 kg: 34 N of thrust is a 1.74 thrust-to-weight.
    let total: f64 = out.thrust_n.iter().sum();
    assert!((total / (2.0 * 9.806_65) - 1.743).abs() < 0.01, "{total}");
}

#[test]
fn armed_rotors_idle_and_disarmed_rotors_stop() {
    let mut motors = QuadMotors::new(X500_MOTOR);
    let idle = settle(&mut motors, [0.0; 4], true);
    assert!((idle.thrust_n[0] - 8.548_58e-6 * 150.0 * 150.0).abs() < 1e-9);
    let stopped = settle(&mut motors, [1.0; 4], false);
    assert!(stopped.thrust_n.iter().all(|&t| t < 1e-9));
}

#[test]
fn the_rotor_lags_its_command() {
    let mut motors = QuadMotors::new(X500_MOTOR);
    settle(&mut motors, [0.0; 4], true);
    // One spin-up time constant reaches 63% of the step in speed.
    let mut out = motors.step([1.0; 4], true, 0.0125);
    let speed = (out.thrust_n[0] / 8.548_58e-6).sqrt();
    assert!(((speed - 150.0) / 850.0 - 0.632).abs() < 0.01, "{speed}");
    out = motors.step([1.0; 4], true, 1.0);
    assert!((out.thrust_n[0] - 8.548_58).abs() < 1e-3);
}

#[test]
fn faster_counter_clockwise_rotors_yaw_the_body_clockwise() {
    let mut motors = QuadMotors::new(X500_MOTOR);
    let out = settle(&mut motors, [0.8, 0.8, 0.6, 0.6], true);
    assert!(out.yaw_torque_nm > 0.0, "{}", out.yaw_torque_nm);
    let nan = settle(&mut motors, [f64::NAN; 4], true);
    assert!(nan.thrust_n.iter().all(|t| t.is_finite()));
}

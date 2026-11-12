use std::time::Duration;

use pilotage_control_feel::{FlightFeelProfile, ValidatedFlightFeelProfile};

use super::{ActivationStep, StickFrame, UplinkPolicy};

fn policy() -> UplinkPolicy {
    let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
        .expect("the legacy profile validates");
    UplinkPolicy::new(profile)
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn stick(throttle: f32) -> StickFrame {
    StickFrame {
        roll: 0.0,
        pitch: 0.0,
        throttle,
        yaw: 0.0,
        current_yaw_rad: 0.3,
        current_pos_ned_m: [0.0; 3],
        current_vel_ned_mps: Some([0.0; 3]),
        current_accel_ned_mps2: Some([0.0; 3]),
    }
}

/// The MAVLink v2 sequence byte of a frame.
fn sequence(frame: &[u8]) -> u8 {
    frame[4]
}

#[test]
fn an_arm_opens_a_quiet_interval_and_a_climb_opens_the_stream() {
    let mut policy = policy();
    let arm = policy.arm(ms(1_000), 0.3);
    assert_eq!(arm[0], 0xFD, "a MAVLink v2 frame");
    assert!(policy.stick_frame(ms(1_100), &stick(1.0)).frame.is_none());
    assert!(!policy.airborne());
    let centered = policy.stick_frame(ms(1_200), &stick(0.0));
    assert!(
        centered.frame.is_none(),
        "a centered throttle keeps the motors idle"
    );
    let climb = policy.stick_frame(ms(1_250), &stick(1.0));
    let frame = climb.frame.expect("a climb opens the stream");
    assert!(policy.airborne());
    assert_eq!(sequence(&frame), sequence(&arm).wrapping_add(1));
}

#[test]
fn a_hold_repeats_at_the_keep_alive_interval_once_airborne() {
    let mut policy = policy();
    assert!(
        policy.keep_holding(ms(0)).is_none(),
        "no hold before take-off"
    );
    policy.arm(ms(0), 0.0);
    policy
        .stick_frame(ms(200), &stick(1.0))
        .frame
        .expect("climbs");
    assert!(policy.keep_holding(ms(300)).is_some());
    assert!(policy.keep_holding(ms(350)).is_none());
    assert!(policy.keep_holding(ms(400)).is_some());
}

#[test]
fn an_activation_that_does_not_arrive_restores_the_heading() {
    let mut policy = policy();
    policy.arm(ms(0), 0.2);
    policy
        .stick_frame(ms(200), &stick(1.0))
        .frame
        .expect("climbs");
    let before = policy.heading();
    let step = policy
        .prepare_control_feel_activation(ms(300), 1.0)
        .expect("a finite heading");
    let ActivationStep::Send { prior, .. } = step else {
        panic!("an airborne activation sends a neutral setpoint: {step:?}");
    };
    assert_eq!(prior, before);
    assert!((policy.heading().heading_rad - 1.0).abs() < 1e-6);
    policy.restore_heading(prior);
    assert_eq!(policy.heading(), before);
}

#[test]
fn the_same_calls_give_the_same_bytes() {
    let run = || {
        let mut policy = policy();
        let mut frames = vec![policy.arm(ms(0), 0.1)];
        for step in 1..20_u64 {
            if let Some(frame) = policy.stick_frame(ms(150 + 20 * step), &stick(0.8)).frame {
                frames.push(frame);
            }
        }
        frames
    };
    assert_eq!(run(), run());
}

#[test]
fn fixed_wing_link_loss_holds_heading_for_sixty_seconds_then_orbits() {
    let mut policy = policy();
    policy.set_fixed_wing_orbit_rate(0.05);
    policy.arm(ms(0), 0.25);
    policy.expire_quiet();
    let _ = policy.fixed_wing_velocity_frame(ms(100), [30.0, 0.0, 0.0], 0.25);

    // Link loss at t = 1_000 ms: holds 0.25 rad right up to t = 61_000 ms (60 s).
    let _ = policy.neutral(ms(1_000));
    for step_ms in (2_000..=61_000).step_by(1_000) {
        let _ = policy.keep_holding(ms(step_ms));
        assert!(
            (policy.heading().heading_rad - 0.25).abs() < 1e-6,
            "heading changed before 60 s at {step_ms} ms: {}",
            policy.heading().heading_rad
        );
    }

    // Past 60 s after link loss (t > 61_000 ms), heading advances at 0.05 rad/s in orbit.
    let mut prev = policy.heading().heading_rad;
    for step_ms in (62_000..=76_000).step_by(1_000) {
        let _ = policy.keep_holding(ms(step_ms));
        let cur = policy.heading().heading_rad;
        assert!(
            cur > prev + 0.04,
            "expected orbit progression at {step_ms} ms: {prev} -> {cur}"
        );
        prev = cur;
    }
}

#![allow(clippy::expect_used, clippy::panic)]

use std::net::UdpSocket;
use std::time::Duration;

use pilotage_control_feel::{FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};

use super::FlightUplink;

fn validated(profile: FlightFeelProfile) -> ValidatedFlightFeelProfile {
    ValidatedFlightFeelProfile::new(profile).expect("valid test profile")
}

fn tuned_profile() -> ValidatedFlightFeelProfile {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.profile_id = "aviate-uplink-test".to_owned();
    profile.mode = FeelMode::Balanced;
    profile.horizontal.neutral.active_enter = 0.10;
    profile.horizontal.neutral.active_exit = 0.05;
    profile.horizontal.curve.deadzone = 0.10;
    profile.horizontal.dynamics.apply_accel = 1.0;
    profile.horizontal.dynamics.apply_jerk = 4.0;
    profile.horizontal.dynamics.release_accel = 2.0;
    profile.horizontal.dynamics.release_jerk = 8.0;
    profile.horizontal.dynamics.reversal_accel = 2.0;
    profile.horizontal.dynamics.reversal_jerk = 8.0;
    profile.direct.tilt_rate_rps = 0.5;
    profile.direct.tilt_accel_rps2 = 1.0;
    profile.direct.thrust_rate_per_s = 1.0;
    profile.direct.thrust_accel_per_s2 = 2.0;
    profile.hold.max_speed_mps = 0.2;
    profile.hold.max_accel_mps2 = 0.4;
    profile.hold.require_accel = true;
    profile.hold.stable_dwell_ms = 40;
    validated(profile)
}

fn ready_uplink(fc: &UdpSocket, profile: ValidatedFlightFeelProfile) -> FlightUplink {
    let mut uplink = FlightUplink::new_with_profile(profile).expect("uplink");
    uplink.set_target(fc.local_addr().expect("fake FC address"));
    uplink.use_manual_clock();
    uplink.send_arm(0.0);
    let mut arm = [0_u8; 128];
    fc.recv_from(&mut arm).expect("arm frame");
    uplink.advance_clock(Duration::from_millis(200));
    uplink
}

fn receive(fc: &UdpSocket) -> [u8; 128] {
    let mut frame = [0_u8; 128];
    fc.recv_from(&mut frame).expect("control frame");
    frame
}

fn field(frame: &[u8; 128], offset: usize) -> f32 {
    f32::from_le_bytes([
        frame[10 + offset],
        frame[11 + offset],
        frame[12 + offset],
        frame[13 + offset],
    ])
}

fn type_mask(frame: &[u8; 128]) -> u16 {
    u16::from_le_bytes([frame[58], frame[59]])
}

fn send_normal(uplink: &mut FlightUplink, pitch: f32, accel: Option<[f32; 3]>) {
    uplink.send_stick_frame(
        0.0,
        pitch,
        0.5,
        0.0,
        core::f32::consts::FRAC_PI_2,
        [10.0, 20.0, -30.0],
        Some([0.0; 3]),
        accel,
    );
}

fn direct_roll(frame: &[u8; 128]) -> f32 {
    let (qw, qx, qy, qz) = (
        field(frame, 4),
        field(frame, 8),
        field(frame, 12),
        field(frame, 16),
    );
    (2.0 * (qw * qx + qy * qz)).atan2(1.0 - 2.0 * (qx * qx + qy * qy))
}

fn direct_yaw(frame: &[u8; 128]) -> f32 {
    let (qw, qx, qy, qz) = (
        field(frame, 4),
        field(frame, 8),
        field(frame, 12),
        field(frame, 16),
    );
    (2.0 * (qw * qz + qx * qy)).atan2(1.0 - 2.0 * (qy * qy + qz * qz))
}

fn direct_pitch(frame: &[u8; 128]) -> f32 {
    let (qw, qx, qy, qz) = (
        field(frame, 4),
        field(frame, 8),
        field(frame, 12),
        field(frame, 16),
    );
    (2.0 * (qw * qy - qz * qx)).clamp(-1.0, 1.0).asin()
}

#[test]
fn compatibility_profile_reproduces_the_velocity_samples() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let legacy = FlightFeelProfile::legacy_compatibility();
    let expected_vertical = -legacy.vertical.curve.apply(0.5) * legacy.envelope.vertical_speed_mps;
    let profile = validated(legacy);
    let mut uplink = ready_uplink(&fc, profile);

    send_normal(&mut uplink, 1.0, None);
    let first = receive(&fc);
    assert!((field(&first, 20) - 5.0 / 60.0).abs() < 1e-5);
    let vertical = field(&first, 24);
    assert!(
        (vertical - expected_vertical).abs() < 1e-5,
        "vertical {vertical}"
    );

    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 1.0, None);
    let second = receive(&fc);
    assert!((field(&second, 20) - (5.0 / 60.0 + 0.1)).abs() < 1e-5);

    uplink.advance_clock(Duration::from_millis(20));
    uplink.send_stick_frame(
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        [10.0, 20.0, -30.0],
        Some([3.0, 0.0, 0.0]),
        None,
    );
    let release = receive(&fc);
    assert_eq!(type_mask(&release), 2503);
    assert!(field(&release, 20).abs() < 1e-6);
}

#[test]
fn compatibility_profile_reproduces_direct_mapping() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let profile = validated(FlightFeelProfile::legacy_compatibility());
    let mut uplink = ready_uplink(&fc, profile);

    assert!(!uplink.send_attitude_frame(0.2, -0.1, 0.8, 0.9));
    let frame = receive(&fc);

    assert!((direct_roll(&frame) - 0.2).abs() < 1e-5);
    assert!((direct_pitch(&frame) + 0.1).abs() < 1e-5);
    assert!((direct_yaw(&frame) - 0.8).abs() < 1e-5);
    assert!((field(&frame, 32) - 0.944).abs() < 1e-5);
}

#[test]
fn normal_control_uses_apply_release_and_hysteresis() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());

    send_normal(&mut uplink, 0.30, None);
    let first = field(&receive(&fc), 20);
    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 0.30, None);
    let entered = field(&receive(&fc), 20);
    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 0.25, None);
    let hysteresis = field(&receive(&fc), 20);
    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 0.15, None);
    let released = field(&receive(&fc), 20);

    assert_eq!(first, 0.0, "zero elapsed time cannot advance the shaper");
    assert!(entered > 0.0);
    assert!(
        hysteresis > entered,
        "exit threshold keeps the input active"
    );
    assert!(released > 0.0 && released <= hysteresis);
}

#[test]
fn hold_capture_needs_acceleration_and_the_complete_dwell() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());
    uplink.send_attitude_frame(0.0, 0.0, 0.0, 1.0);
    receive(&fc);

    uplink.send_stick_frame(
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        [10.0, 20.0, -30.0],
        Some([0.0; 3]),
        None,
    );
    assert_eq!(type_mask(&receive(&fc)), 2503, "acceleration is missing");

    for expected_mask in [2503, 2552] {
        uplink.advance_clock(Duration::from_millis(20));
        uplink.send_stick_frame(
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            [10.0, 20.0, -30.0],
            Some([0.0; 3]),
            Some([0.2, 0.0, 0.0]),
        );
        assert_eq!(type_mask(&receive(&fc)), expected_mask);
    }
}

#[test]
fn direct_mode_and_link_reset_clear_temporal_state() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());

    uplink.send_attitude_frame(0.5, 0.0, 0.0, 1.0);
    let initial_roll = direct_roll(&receive(&fc));
    uplink.advance_clock(Duration::from_millis(100));
    uplink.send_attitude_frame(0.5, 0.0, 0.0, 1.0);
    let continued_roll = direct_roll(&receive(&fc));
    assert!(continued_roll > initial_roll);

    send_normal(&mut uplink, 0.0, None);
    receive(&fc);
    uplink.send_attitude_frame(0.5, 0.0, 0.0, 1.0);
    let mode_reset_roll = direct_roll(&receive(&fc));
    assert!((mode_reset_roll - initial_roll).abs() < 1e-5);

    uplink.advance_clock(Duration::from_millis(100));
    uplink.send_attitude_frame(0.5, 0.0, 0.0, 1.0);
    receive(&fc);
    uplink.clear_hold_state();
    uplink.send_attitude_frame(0.5, 0.0, 0.0, 1.0);
    let link_reset_roll = direct_roll(&receive(&fc));
    assert!((link_reset_roll - initial_roll).abs() < 1e-5);
}

#[test]
fn vehicle_reset_reseeds_heading_from_the_next_measurement() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());
    uplink.reset_for_vehicle_reset();

    uplink.send_stick_frame(0.0, 0.0, 0.5, 0.0, 1.2, [0.0; 3], Some([0.0; 3]), None);
    let frame = receive(&fc);

    assert!((field(&frame, 40) - 1.2).abs() < 1e-5);
}

#[test]
fn direct_entry_and_mode_switch_keep_thrust_above_the_profile_floor() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());

    assert!(uplink.send_attitude_frame(0.5, 0.0, 1.0, 1.0));
    let first = receive(&fc);
    assert!(field(&first, 32) >= 0.762);

    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 0.5, None);
    receive(&fc);
    uplink.advance_clock(Duration::from_millis(20));
    assert!(uplink.send_attitude_frame(0.5, 0.0, 1.0, 1.0));
    let switched = receive(&fc);
    assert!(field(&switched, 32) >= 0.30);
}

#[test]
fn direct_entry_starts_from_the_measured_attitude() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());

    assert!(!uplink.send_attitude_frame_seeded(0.4, -0.3, 0.0, 1.0, [0.2, -0.1, 0.0]));
    let first = receive(&fc);
    assert!((direct_roll(&first) - 0.2).abs() < 1e-5);
    assert!((direct_pitch(&first) + 0.1).abs() < 1e-5);
    assert!(field(&first, 32) >= 0.762);
}

#[test]
fn normal_mode_switch_starts_from_the_measured_velocity() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());
    uplink.send_attitude_frame(0.0, 0.0, 0.0, 1.0);
    receive(&fc);

    uplink.send_stick_frame(
        0.0,
        0.0,
        0.5,
        0.0,
        0.0,
        [0.0; 3],
        Some([0.4, -0.2, -0.1]),
        None,
    );
    let first = receive(&fc);

    assert!((field(&first, 16) - 0.4).abs() < 1e-5);
    assert!((field(&first, 20) + 0.2).abs() < 1e-5);
    assert!((field(&first, 24) + 0.1).abs() < 1e-5);
}

#[test]
fn direct_heading_is_limited_by_the_advertised_rate() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());

    assert!(uplink.send_attitude_frame(0.0, 0.0, 1.0, 1.0));
    let first = receive(&fc);
    assert!(direct_yaw(&first).abs() < 1e-5);
    uplink.advance_clock(Duration::from_millis(20));
    assert!(uplink.send_attitude_frame(0.0, 0.0, 1.0, 1.0));
    let second = receive(&fc);
    assert!((direct_yaw(&second) - 0.018).abs() < 1e-4);
}

#[test]
fn normal_release_preserves_rate_continuity_and_converges() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());
    let mut before_prior = 0.0;
    let mut prior = 0.0;
    for _ in 0..30 {
        uplink.advance_clock(Duration::from_millis(20));
        send_normal(&mut uplink, 1.0, None);
        before_prior = prior;
        prior = field(&receive(&fc), 20).abs();
    }
    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 0.0, None);
    let first_release = field(&receive(&fc), 20).abs();
    let apply_step = prior - before_prior;
    let release_step = first_release - prior;
    let maximum_step_change = 8.0 * 0.02 * 0.02;
    assert!(
        first_release > prior,
        "release cannot reset a positive rate"
    );
    assert!((release_step - apply_step).abs() <= maximum_step_change + 1e-5);

    let mut value = first_release;
    for _ in 0..60 {
        uplink.advance_clock(Duration::from_millis(20));
        send_normal(&mut uplink, 0.0, None);
        value = field(&receive(&fc), 20).abs();
    }
    assert!(value < 1e-5, "release ended at {value}");
}

#[test]
fn non_finite_direct_inputs_are_constrained_to_finite_output() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());

    uplink.send_attitude_frame(0.0, 0.0, 0.0, 1.0);
    receive(&fc);
    uplink.advance_clock(Duration::from_millis(20));
    assert!(uplink.send_attitude_frame(f32::NAN, f32::INFINITY, f32::NAN, f32::NAN));
    let frame = receive(&fc);
    for offset in [4, 8, 12, 16, 32] {
        assert!(field(&frame, offset).is_finite());
    }
    assert!(field(&frame, 32) >= 0.30);
}

#[test]
fn direct_output_stays_inside_the_advertised_bounds() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let profile = tuned_profile();
    let tilt_limit = profile.profile().envelope.direct_tilt_rad;
    let minimum_thrust = profile.profile().envelope.direct_min_thrust;
    let mut uplink = ready_uplink(&fc, profile);

    for _ in 0..40 {
        uplink.advance_clock(Duration::from_millis(20));
        assert!(uplink.send_attitude_frame(10.0, -10.0, 0.0, 2.0));
        let frame = receive(&fc);
        assert!(direct_roll(&frame).abs() <= tilt_limit + 1e-5);
        assert!(direct_pitch(&frame).abs() <= tilt_limit + 1e-5);
        assert!((minimum_thrust..=1.0).contains(&field(&frame, 32)));
    }
}

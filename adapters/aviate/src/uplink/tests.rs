#![allow(clippy::expect_used, clippy::panic)]

use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pilotage_adapter_api::VehicleAdapter;
use pilotage_control_feel::{FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_protocol::VehicleId;

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
    profile.horizontal.dynamics.apply_accel = 1.0;
    profile.horizontal.dynamics.apply_jerk = 4.0;
    profile.horizontal.dynamics.release_accel = 2.0;
    profile.horizontal.dynamics.release_jerk = 8.0;
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
    let profile = validated(FlightFeelProfile::legacy_compatibility());
    let mut uplink = ready_uplink(&fc, profile);

    send_normal(&mut uplink, 1.0, None);
    let first = receive(&fc);
    assert!((field(&first, 20) - 5.0 / 60.0).abs() < 1e-5);
    let vertical = field(&first, 24);
    assert!((vertical + 0.75).abs() < 1e-5, "vertical {vertical}");

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

    assert!(!uplink.send_attitude_frame(0.2, -0.1, 0.0, 0.9));
    let frame = receive(&fc);

    assert!((direct_roll(&frame) - 0.2).abs() < 1e-5);
    assert!((direct_pitch(&frame) + 0.1).abs() < 1e-5);
    assert!((field(&frame, 32) - 0.944).abs() < 1e-5);
}

#[test]
fn normal_control_uses_apply_release_and_hysteresis() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());

    send_normal(&mut uplink, 0.20, None);
    let first = field(&receive(&fc), 20);
    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 0.20, None);
    let entered = field(&receive(&fc), 20);
    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 0.07, None);
    let hysteresis = field(&receive(&fc), 20);
    uplink.advance_clock(Duration::from_millis(20));
    send_normal(&mut uplink, 0.04, None);
    let released = field(&receive(&fc), 20);

    assert_eq!(first, 0.0, "zero elapsed time cannot advance the shaper");
    assert!(entered > 0.0);
    assert!(
        hysteresis > entered,
        "exit threshold keeps the input active"
    );
    assert!(released > 0.0 && released < hysteresis);
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
fn adapter_advertises_the_injected_profile_envelope() {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.profile_id = "advertised-envelope".to_owned();
    profile.envelope.horizontal_speed_mps = 4.0;
    profile.envelope.vertical_speed_mps = 2.0;
    profile.envelope.yaw_rate_rps = 0.7;
    profile.envelope.direct_tilt_rad = 0.4;
    let uplink = FlightUplink::new_with_profile(validated(profile)).expect("uplink");
    let state = Arc::new(Mutex::new(pilotage_mavlink::link::LinkState::default()));
    let adapter = crate::AviateAdapter::from_state(VehicleId::new(1), state).with_uplink(uplink);

    let capabilities = adapter.capabilities();
    let scopes = &capabilities.vehicles[0].scopes;
    let normal = scopes
        .iter()
        .find(|scope| scope.scope.as_str() == crate::adapter::FLIGHT_SCOPE)
        .expect("normal scope");
    let direct = scopes
        .iter()
        .find(|scope| scope.scope.as_str() == crate::adapter::DIRECT_SCOPE)
        .expect("direct scope");
    assert_eq!(normal.intents[0].max_linear, 4.0);
    assert_eq!(normal.intents[0].max_vertical, 2.0);
    assert_eq!(normal.intents[0].max_angular, 0.7);
    assert_eq!(direct.intents[0].max_angular, 0.4);
    assert_eq!(direct.intents[0].max_yaw_rate, 0.7);
    assert!(capabilities.adapter_version.contains("feel-schema=1"));
    assert!(
        capabilities
            .adapter_version
            .contains("feel-id=advertised-envelope")
    );
    assert!(capabilities.adapter_version.contains("feel-sha256="));
}

#[test]
fn direct_entry_and_mode_switch_keep_thrust_above_the_profile_floor() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());

    assert!(uplink.send_attitude_frame(0.5, 0.0, 1.0, 1.0));
    let first = receive(&fc);
    assert!(field(&first, 32) >= 0.30);

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
    assert!(field(&first, 32) >= 0.30);
}

#[test]
fn normal_entry_starts_from_the_measured_velocity() {
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
fn normal_release_never_increases_the_velocity_command() {
    let fc = UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut uplink = ready_uplink(&fc, tuned_profile());
    let mut prior = 0.0;
    for _ in 0..30 {
        uplink.advance_clock(Duration::from_millis(20));
        send_normal(&mut uplink, 1.0, None);
        prior = field(&receive(&fc), 20).abs();
    }
    for _ in 0..30 {
        uplink.advance_clock(Duration::from_millis(20));
        send_normal(&mut uplink, 0.0, None);
        let value = field(&receive(&fc), 20).abs();
        assert!(value <= prior + 1e-5, "{value} followed {prior}");
        prior = value;
    }
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

//! Golden coverage for the physical-input and control-feel layers.

use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_control_feel::{AxisDynamics, FeelMode, FlightFeelProfile};
use pilotage_control_web::{
    AuthorityEvent, AuthorityScope, ControlRuntime, DeviceStage, Mode, ProfileRuntime, RawSample,
    SessionState,
};
use pilotage_input::ProfileLayer;
use pilotage_protocol::{ButtonEdge, ControlIntent, LogicalAxisId, LogicalButtonId, VehicleId};
use serde::Deserialize;

use super::super::{ARM_BUTTON, AviateAdapter};
use super::fixtures::{flight_frame, state_with};

const NOISE_PROFILE: &str = r#"{
  "schema_version": 1,
  "revision": 2,
  "device": { "vendor_id": 4617, "product_id": 20308, "product": "Test radio" },
  "description": "Physical calibration and noise suppression only.",
  "axes": [{
    "source_index": 0,
    "logical": "slot2",
    "invert": false,
    "deadzone": 0.05,
    "expo": 0.0,
    "calibration": { "min": -1.0, "center": 0.0, "max": 1.0 }
  }],
  "buttons": []
}"#;

const LEGACY_HID_MAVLINK_GOLDEN: &str =
    include_str!("../../../tests/fixtures/legacy-hid-mavlink-v1.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyGolden {
    device_id: String,
    raw_axes: Vec<f32>,
    expected_frames: Vec<ExpectedFrame>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFrame {
    north_mps: f32,
    east_mps: f32,
    down_mps: f32,
    yaw_rad: f32,
}

fn feel_profile() -> pilotage_control_feel::ValidatedFlightFeelProfile {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.profile_id = "hid-typed-feel-golden".to_owned();
    profile.mode = FeelMode::Balanced;
    profile.horizontal.curve.center_expo = 0.5;
    profile.horizontal.curve.outer_expo = 0.5;
    profile.horizontal.dynamics = AxisDynamics {
        apply_accel: 100_000.0,
        release_accel: 100_000.0,
        apply_jerk: 100_000.0,
        release_jerk: 100_000.0,
        reversal_accel: 100_000.0,
        reversal_jerk: 100_000.0,
    };
    pilotage_control_feel::ValidatedFlightFeelProfile::new(profile).expect("valid feel profile")
}

fn canonical_radio_sample() -> RawSample {
    let mut stage = DeviceStage::new();
    assert!(stage.add_profile(ProfileLayer::Session, NOISE_PROFILE.as_bytes()));
    stage.select_pad("RadioMaster Pocket (Vendor: 1209 Product: 4f54)");
    let mut sample = RawSample::default();
    stage.pad_sample(&[0.525, 0.0, 1.0, 0.0], &[], &mut sample);
    assert!((sample.axes[2] - 0.5).abs() < 1e-6);
    sample
}

fn radiomaster_sample(golden: &LegacyGolden) -> (RawSample, [u8; 32]) {
    let mut stage = DeviceStage::new();
    stage.select_pad(&golden.device_id);
    let mut sample = RawSample::default();
    stage.pad_sample(&golden.raw_axes, &[], &mut sample);
    let digest = stage.pad_digest().expect("device profile digest");
    (sample, digest)
}

fn typed_motion_frame(sample: &RawSample) -> pilotage_protocol::ScopedControlFrame {
    let mut runtime = ControlRuntime::new();
    let profile = ProfileRuntime::compile(pilotage_control_web::DEFAULT_PROFILE_BYTES)
        .expect("linear flight profile");
    runtime.activate(profile);
    runtime.begin_session();
    runtime.authority_event(
        AuthorityScope::Motion,
        AuthorityEvent::LeaseGranted { generation: 1 },
    );
    let plan = runtime.evaluate(
        sample,
        &SessionState {
            now_ms: 1_000.0,
            mode: Mode::QuadPilot,
            connected: true,
            input_lost: false,
        },
    );
    let axes = plan
        .motion
        .expect("typed motion")
        .axes()
        .iter()
        .map(|(axis, value)| (LogicalAxisId::new(*axis), *value))
        .collect();
    flight_frame(axes, vec![])
}

fn field(frame: &[u8; 128], offset: usize) -> f32 {
    f32::from_le_bytes([
        frame[10 + offset],
        frame[11 + offset],
        frame[12 + offset],
        frame[13 + offset],
    ])
}

fn assert_velocity_frame(frame: &[u8; 128], expected: &ExpectedFrame) {
    for (actual, expected) in [
        (field(frame, 16), expected.north_mps),
        (field(frame, 20), expected.east_mps),
        (field(frame, 24), expected.down_mps),
        (field(frame, 40), expected.yaw_rad),
    ] {
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }
}

#[test]
fn legacy_hid_to_mavlink_commands_match_the_frozen_golden() {
    let golden: LegacyGolden =
        serde_json::from_str(LEGACY_HID_MAVLINK_GOLDEN).expect("golden fixture");
    let (sample, device_digest) = radiomaster_sample(&golden);
    let typed = typed_motion_frame(&sample);
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let legacy = FlightFeelProfile::legacy_compatibility();
    assert_eq!(
        legacy.bindings.device_profile_sha256.as_bytes(),
        &device_digest
    );
    let profile =
        pilotage_control_feel::ValidatedFlightFeelProfile::new(legacy).expect("legacy profile");
    let mut uplink = crate::FlightUplink::new_with_profile(profile).expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    let state = state_with(Duration::ZERO, Duration::ZERO);
    let mut adapter = AviateAdapter::from_state(VehicleId::new(1), state).with_uplink(uplink);
    let mut wire = [0_u8; 128];

    for (index, expected) in golden.expected_frames.iter().enumerate() {
        if index > 0 {
            adapter
                .uplink_mut()
                .expect("uplink")
                .advance_clock(Duration::from_millis(20));
        }
        assert_eq!(
            adapter.apply_control(&typed).disposition,
            Disposition::Accepted
        );
        fc.recv_from(&mut wire).expect("velocity frame");
        assert_velocity_frame(&wire, expected);
    }
}

#[test]
fn hid_is_linear_until_the_control_feel_curve() {
    let typed = typed_motion_frame(&canonical_radio_sample());
    let Some(ControlIntent::Velocity(velocity)) = typed.intent else {
        panic!("velocity intent");
    };
    assert!((velocity.vy - 1.5).abs() < 1e-6);

    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let profile = feel_profile();
    let expected = -3.0 * profile.profile().horizontal.curve.apply(0.5);
    let mut uplink = crate::FlightUplink::new_with_profile(profile).expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    let state = state_with(Duration::ZERO, Duration::ZERO);
    state
        .lock()
        .expect("state")
        .kinematics
        .as_mut()
        .expect("kinematics")
        .vel_ned_mps = [0.0; 3];
    let mut adapter = AviateAdapter::from_state(VehicleId::new(1), state).with_uplink(uplink);
    adapter.apply_control(&flight_frame(
        vec![],
        vec![(LogicalButtonId::new(ARM_BUTTON), ButtonEdge::Pressed)],
    ));
    let mut wire = [0_u8; 128];
    fc.recv_from(&mut wire).expect("arm frame");
    adapter
        .uplink_mut()
        .expect("uplink")
        .advance_clock(Duration::from_millis(200));

    assert_eq!(
        adapter.apply_control(&typed).disposition,
        Disposition::Accepted
    );
    fc.recv_from(&mut wire).expect("first setpoint");
    adapter
        .uplink_mut()
        .expect("uplink")
        .advance_clock(Duration::from_millis(100));
    assert_eq!(
        adapter.apply_control(&typed).disposition,
        Disposition::Accepted
    );
    fc.recv_from(&mut wire).expect("shaped setpoint");
    adapter
        .uplink_mut()
        .expect("uplink")
        .advance_clock(Duration::from_millis(100));
    assert_eq!(
        adapter.apply_control(&typed).disposition,
        Disposition::Accepted
    );
    fc.recv_from(&mut wire).expect("settled setpoint");

    let actual = field(&wire, 16);
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, got {actual}"
    );
}

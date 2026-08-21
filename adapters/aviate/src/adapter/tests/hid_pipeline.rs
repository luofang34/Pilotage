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

fn feel_profile() -> pilotage_control_feel::ValidatedFlightFeelProfile {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.profile_id = "hid-typed-feel-golden".to_owned();
    profile.mode = FeelMode::Balanced;
    profile.horizontal.curve.expo = 0.5;
    profile.horizontal.dynamics = AxisDynamics {
        apply_accel: 100_000.0,
        release_accel: 100_000.0,
        apply_jerk: 100_000.0,
        release_jerk: 100_000.0,
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
    let mut uplink = crate::FlightUplink::new_with_profile(feel_profile()).expect("uplink");
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

    let expected = -3.0 * 0.5_f32.powf(1.5);
    let actual = field(&wire, 16);
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, got {actual}"
    );
}

//! Control-feel artifact and activation transaction tests.

use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_control_feel::{FeelDigest, FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_protocol::{LogicalAxisId, VehicleId};

use super::super::{AviateAdapter, PITCH_AXIS};
use super::fixtures::{flight_frame, state_with};

fn candidate(id: &str) -> ValidatedFlightFeelProfile {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.profile_id = id.to_owned();
    profile.mode = FeelMode::Balanced;
    profile.horizontal.curve.center_expo = 0.25;
    profile.horizontal.curve.outer_expo = 0.1;
    profile.horizontal.curve.outer_start = 0.7;
    profile.horizontal.neutral.active_exit = 0.005;
    ValidatedFlightFeelProfile::new(profile).expect("valid candidate")
}

fn adapter_with_fc(fc: &std::net::UdpSocket) -> AviateAdapter {
    let mut uplink = crate::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    )
    .with_uplink(uplink)
}

fn active_digest(adapter: &AviateAdapter) -> [u8; 32] {
    adapter
        .capabilities()
        .control_feel
        .expect("active identity")
        .profile_sha256
}

fn neutral_frame() -> pilotage_protocol::ScopedControlFrame {
    flight_frame(vec![], vec![])
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
fn the_checked_default_is_the_canonical_compatibility_artifact() {
    let parsed =
        ValidatedFlightFeelProfile::from_json_str(crate::ALIA250_DEFAULT_CONTROL_FEEL_JSON)
            .expect("checked artifact");
    let canonical = serde_json::to_string(parsed.profile()).expect("canonical JSON");
    assert_eq!(parsed.profile(), &FlightFeelProfile::legacy_compatibility());
    assert_eq!(
        canonical,
        crate::ALIA250_DEFAULT_CONTROL_FEEL_JSON.trim_end()
    );
}

#[test]
fn uplink_refuses_a_changed_envelope_and_advertises_the_required_artifact() {
    let mut changed = FlightFeelProfile::legacy_compatibility();
    changed.envelope.horizontal_speed_mps = 4.0;
    let changed = ValidatedFlightFeelProfile::new(changed).expect("generic profile");
    assert!(crate::FlightUplink::new_with_profile(changed).is_err());

    let required = FlightFeelProfile::legacy_compatibility();
    let bindings = required.bindings;
    let uplink = crate::FlightUplink::new_with_profile(
        ValidatedFlightFeelProfile::new(required).expect("required profile"),
    )
    .expect("uplink");
    let adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    )
    .with_uplink(uplink);

    let capabilities = adapter.capabilities();
    let scopes = &capabilities.vehicles[0].scopes;
    let normal = scopes
        .iter()
        .find(|scope| scope.scope.as_str() == super::super::FLIGHT_SCOPE)
        .expect("normal scope");
    let direct = scopes
        .iter()
        .find(|scope| scope.scope.as_str() == super::super::DIRECT_SCOPE)
        .expect("direct scope");
    assert_eq!(normal.intents[0].max_linear, 3.0);
    assert_eq!(normal.intents[0].max_vertical, 1.5);
    assert_eq!(normal.intents[0].max_angular, 0.9);
    assert_eq!(direct.intents[0].max_angular, 0.6);
    assert_eq!(direct.intents[0].max_yaw_rate, 0.9);
    assert_eq!(
        capabilities.adapter_version,
        format!(
            "{};control-clock=system-monotonic-v1",
            env!("CARGO_PKG_VERSION")
        )
    );
    let feel = capabilities.control_feel.expect("typed feel identity");
    assert_eq!(
        feel.device_profile_sha256,
        *bindings.device_profile_sha256.as_bytes()
    );
    assert_eq!(
        feel.flight_controller_sha256,
        *bindings.flight_controller_sha256.as_bytes()
    );
}

#[test]
fn normal_takeoff_starts_at_the_profile_climb_floor() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
        .expect("required profile");
    let minimum_climb =
        profile.profile().envelope.takeoff_input * profile.profile().envelope.vertical_speed_mps;
    let mut uplink = crate::FlightUplink::new_with_profile(profile).expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    uplink.send_arm(0.0);
    let mut frame = [0_u8; 128];
    fc.recv_from(&mut frame).expect("arm frame");
    uplink.advance_clock(Duration::from_millis(200));

    uplink.send_stick_frame(0.0, 0.0, 0.5, 0.0, 0.0, [0.0; 3], Some([0.0; 3]), None);
    fc.recv_from(&mut frame).expect("takeoff frame");
    let climb = -field(&frame, 24);

    assert!(
        climb + 1e-6 >= minimum_climb,
        "climb {climb}, minimum {minimum_climb}"
    );
}

#[test]
fn activation_waits_for_union_neutral_and_sends_neutral_first() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let mut adapter = adapter_with_fc(&fc);
    let before = active_digest(&adapter);
    let next = candidate("alia250-balanced-test");
    let expected = *FeelDigest::calculate(&next).expect("digest").as_bytes();
    adapter.stage_control_feel(next).expect("stage candidate");

    let active = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
    assert_eq!(
        adapter.apply_control(&active).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), before);

    let neutral_only_for_active =
        flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.08)], vec![]);
    adapter.apply_control(&neutral_only_for_active);
    assert_eq!(active_digest(&adapter), before);

    adapter
        .uplink_mut()
        .expect("uplink")
        .seed_hold_for_test([1.0, 2.0, 3.0]);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), expected);
    assert!(!adapter.uplink_hold_captured());

    let mut frame = [0_u8; 128];
    fc.recv_from(&mut frame).expect("activation neutral frame");
    for offset in [16, 20, 24] {
        assert!(field(&frame, offset).abs() < f32::EPSILON);
    }
}

#[test]
fn rejection_preserves_active_and_rollback_restores_the_complete_artifact() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    let original = active_digest(&adapter);

    let mut unsafe_profile = FlightFeelProfile::legacy_compatibility();
    unsafe_profile.envelope.horizontal_speed_mps = 4.0;
    let unsafe_profile = ValidatedFlightFeelProfile::new(unsafe_profile).expect("generic profile");
    assert!(adapter.stage_control_feel(unsafe_profile).is_err());
    assert_eq!(active_digest(&adapter), original);

    adapter
        .stage_control_feel(candidate("alia250-balanced-rollback"))
        .expect("stage candidate");
    adapter.apply_control(&neutral_frame());
    assert_ne!(active_digest(&adapter), original);
    assert!(adapter.stage_control_feel_rollback());
    adapter.apply_control(&neutral_frame());
    assert_eq!(active_digest(&adapter), original);
}

#[test]
fn a_rejected_neutral_output_preserves_active_and_pending_artifacts() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let mut adapter = adapter_with_fc(&fc);
    let original = active_digest(&adapter);
    let next = candidate("alia250-balanced-send-retry");
    let expected = *FeelDigest::calculate(&next).expect("digest").as_bytes();
    adapter.stage_control_feel(next).expect("stage candidate");
    adapter
        .uplink_mut()
        .expect("uplink")
        .set_target("[::1]:9".parse().expect("IPv6 target"));

    let failed = adapter.apply_control(&neutral_frame());
    assert!(matches!(failed.disposition, Disposition::Rejected(_)));
    assert_eq!(active_digest(&adapter), original);

    adapter
        .uplink_mut()
        .expect("uplink")
        .set_target(fc.local_addr().expect("FC address"));
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), expected);
    let mut frame = [0_u8; 128];
    fc.recv_from(&mut frame).expect("activation neutral frame");
}

//! Control-feel artifact and activation transaction tests.

mod neutral_boundary;
mod notifications;

use std::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_control_feel::{FeelDigest, FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_protocol::{ButtonEdge, LogicalAxisId, LogicalButtonId, ScopeId, VehicleId};

use super::super::{
    ARM_BUTTON, AviateAdapter, AviateProfile, DIRECT_SCOPE, FLIGHT_SCOPE, PITCH_AXIS, THROTTLE_AXIS,
};
use super::fixtures::{direct_frame, flight_frame, state_with};

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

fn airborne_adapter_with_fc(fc: &std::net::UdpSocket) -> AviateAdapter {
    let mut adapter = adapter_with_fc(fc);
    make_airborne(&mut adapter, fc);
    adapter
}

fn make_airborne(adapter: &mut AviateAdapter, fc: &std::net::UdpSocket) {
    fc.set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let arm = flight_frame(
        vec![],
        vec![(LogicalButtonId::new(ARM_BUTTON), ButtonEdge::Pressed)],
    );
    assert_eq!(
        adapter.apply_control(&arm).disposition,
        Disposition::Accepted
    );
    let mut frame = [0_u8; 128];
    fc.recv_from(&mut frame).expect("arm frame");
    adapter
        .uplink_mut()
        .expect("uplink")
        .advance_clock(Duration::from_millis(200));
    let climb = flight_frame(vec![(LogicalAxisId::new(THROTTLE_AXIS), 0.5)], vec![]);
    assert_eq!(
        adapter.apply_control(&climb).disposition,
        Disposition::Accepted
    );
    fc.recv_from(&mut frame).expect("takeoff frame");
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

fn assert_no_frame(fc: &std::net::UdpSocket) {
    let mut frame = [0_u8; 128];
    let error = fc.recv_from(&mut frame).expect_err("no FC frame");
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
}

fn receive_frame(fc: &std::net::UdpSocket, detail: &str) -> [u8; 128] {
    let mut frame = [0_u8; 128];
    fc.recv_from(&mut frame).expect(detail);
    frame
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
    assert_eq!(capabilities.adapter_version, env!("CARGO_PKG_VERSION"));
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
    let mut adapter = airborne_adapter_with_fc(&fc);
    let before = active_digest(&adapter);
    let next = candidate("alia250-balanced-test");
    let expected = *FeelDigest::calculate(&next).expect("digest").as_bytes();
    adapter.stage_control_feel(next).expect("stage candidate");

    let active = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
    assert_eq!(
        adapter.apply_control(&active).disposition,
        Disposition::Accepted
    );
    receive_frame(&fc, "active response frame");
    assert_eq!(active_digest(&adapter), before);

    let neutral_only_for_active =
        flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.08)], vec![]);
    assert_eq!(
        adapter.apply_control(&neutral_only_for_active).disposition,
        Disposition::Accepted
    );
    receive_frame(&fc, "partial-neutral response frame");
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

    let frame = receive_frame(&fc, "activation neutral frame");
    for offset in [16, 20, 24] {
        assert!(field(&frame, offset).abs() < f32::EPSILON);
    }
}

#[test]
fn a_fresh_ground_neutral_activation_commits_without_an_fc_packet() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_millis(20)))
        .expect("read timeout");
    let mut adapter = adapter_with_fc(&fc);
    let next = candidate("alia250-balanced-ground");
    let expected = *FeelDigest::calculate(&next).expect("digest").as_bytes();
    adapter.stage_control_feel(next).expect("stage candidate");

    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), expected);
    assert_no_frame(&fc);
}

#[test]
fn rejection_preserves_active_and_rollback_restores_the_complete_artifact() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = airborne_adapter_with_fc(&fc);
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
    let state = state_with(Duration::ZERO, Duration::ZERO);
    let mut uplink = crate::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    let mut adapter =
        AviateAdapter::from_state(VehicleId::new(1), state.clone()).with_uplink(uplink);
    make_airborne(&mut adapter, &fc);
    let original = active_digest(&adapter);
    let prior_heading = adapter
        .uplink_mut()
        .expect("uplink")
        .heading_state_for_test();
    state
        .lock()
        .expect("state lock")
        .attitude
        .as_mut()
        .expect("attitude")
        .quat_wxyz = [1.0, 0.0, 0.0, 0.0];
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
    assert_eq!(
        adapter
            .uplink_mut()
            .expect("uplink")
            .heading_state_for_test(),
        prior_heading
    );

    adapter
        .uplink_mut()
        .expect("uplink")
        .set_target(fc.local_addr().expect("FC address"));
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), expected);
    let (heading, valid) = adapter
        .uplink_mut()
        .expect("uplink")
        .heading_state_for_test();
    assert!(valid);
    assert!(heading.abs() < f32::EPSILON);
    let mut frame = [0_u8; 128];
    fc.recv_from(&mut frame).expect("activation neutral frame");
}

#[test]
fn a_neutral_intent_in_the_wrong_scope_does_not_activate_a_candidate() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = airborne_adapter_with_fc(&fc);
    let original = active_digest(&adapter);
    let velocity_candidate = candidate("alia250-balanced-scope-velocity");
    let velocity_digest = *FeelDigest::calculate(&velocity_candidate)
        .expect("velocity candidate digest")
        .as_bytes();
    adapter
        .stage_control_feel(velocity_candidate)
        .expect("stage velocity candidate");

    let mut velocity_in_direct = neutral_frame();
    velocity_in_direct.scope = ScopeId::new(DIRECT_SCOPE);
    assert!(matches!(
        adapter.apply_control(&velocity_in_direct).disposition,
        Disposition::Rejected(_)
    ));
    assert_eq!(active_digest(&adapter), original);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), velocity_digest);

    let attitude_candidate = candidate("alia250-balanced-scope-attitude");
    let attitude_digest = *FeelDigest::calculate(&attitude_candidate)
        .expect("attitude candidate digest")
        .as_bytes();
    adapter
        .stage_control_feel(attitude_candidate)
        .expect("stage attitude candidate");
    let mut attitude_in_velocity = direct_frame(0.0, 0.0, 0.0, 0.5);
    attitude_in_velocity.scope = ScopeId::new(FLIGHT_SCOPE);
    assert!(matches!(
        adapter.apply_control(&attitude_in_velocity).disposition,
        Disposition::Rejected(_)
    ));
    assert_eq!(active_digest(&adapter), velocity_digest);
    assert_eq!(
        adapter
            .apply_control(&direct_frame(0.0, 0.0, 0.0, 0.5))
            .disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), attitude_digest);
}

#[test]
fn activation_waits_for_the_complete_arm_quiet_window() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_millis(20)))
        .expect("read timeout");
    let mut adapter = adapter_with_fc(&fc);
    let original = active_digest(&adapter);
    let arm = flight_frame(
        vec![],
        vec![(LogicalButtonId::new(ARM_BUTTON), ButtonEdge::Pressed)],
    );
    assert_eq!(
        adapter.apply_control(&arm).disposition,
        Disposition::Accepted
    );
    let mut frame = [0_u8; 128];
    fc.recv_from(&mut frame).expect("arm frame");
    let next = candidate("alia250-balanced-arm-quiet");
    let expected = *FeelDigest::calculate(&next).expect("digest").as_bytes();
    adapter.stage_control_feel(next).expect("stage candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), original);
    assert_no_frame(&fc);

    adapter
        .uplink_mut()
        .expect("uplink")
        .advance_clock(Duration::from_millis(200));
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), expected);
    assert_no_frame(&fc);
}

#[tokio::test]
async fn a_physical_start_refuses_an_unqualified_control_feel_artifact() {
    let result = AviateAdapter::start_with_control_feel(
        VehicleId::new(1),
        AviateProfile::Physical,
        crate::LinkConfig::physical(),
        candidate("alia250-balanced-physical-start"),
    )
    .await;
    assert!(matches!(
        result,
        Err(crate::AviateAdapterError::PhysicalControlFeelOverride { .. })
    ));
}

#[test]
fn a_physical_adapter_refuses_runtime_stage_and_rollback() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    adapter
        .stage_control_feel(candidate("alia250-balanced-prior"))
        .expect("stage prior candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    adapter.profile = AviateProfile::Physical;

    assert!(matches!(
        adapter.stage_control_feel(candidate("alia250-balanced-physical-stage")),
        Err(crate::AviateAdapterError::PhysicalControlFeelOverride { .. })
    ));
    assert!(!adapter.stage_control_feel_rollback());
}

#[test]
fn activation_with_an_unavailable_pose_preserves_the_pending_candidate() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_millis(20)))
        .expect("read timeout");
    let mut uplink = crate::FlightUplink::new().expect("uplink");
    uplink.set_target(fc.local_addr().expect("FC address"));
    uplink.use_manual_clock();
    let state = state_with(Duration::ZERO, Duration::ZERO);
    state.lock().expect("state lock").attitude = None;
    let mut adapter = AviateAdapter::from_state(VehicleId::new(1), state).with_uplink(uplink);
    let original = active_digest(&adapter);
    adapter
        .stage_control_feel(candidate("alia250-balanced-missing-pose"))
        .expect("stage candidate");

    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Rejected(pilotage_adapter_api::RejectReason::MeasurementUnavailable)
    );
    assert_eq!(active_digest(&adapter), original);
    assert_no_frame(&fc);
}

#[test]
fn every_shaped_operator_mode_installs_after_its_neutral_dwell() {
    // The shaped modes are the answer to a command law that steps, and a
    // profile the adapter will not take is no answer at all. Each one is
    // staged on a flying vehicle and has to reach the active artifact.
    //
    // The law this replaces had no dwell, so activation was instant: one frame
    // at centre and the law changed. A shaped mode holds a quiet band for a
    // stated time before it calls an input neutral, which is what keeps a
    // resting hand from commanding — and the same rule governs the boundary
    // the new law arrives at, so a stick crossing centre in passing does not
    // change the law underneath it.
    //
    // The dwell accumulates across frames rather than from one long gap: a
    // control loop reports continuously, and a single sample after a silence
    // says nothing about where the stick was during it.
    for mode in [FeelMode::Precision, FeelMode::Balanced, FeelMode::Agile] {
        let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
        fc.set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout");
        let mut adapter = adapter_with_fc(&fc);
        let before = active_digest(&adapter);

        // A shaped profile keeps the artifact bindings and the demand envelope
        // of the law it replaces, so it is installable on the vehicle the
        // adapter was started with rather than describing a different one.
        let profile = FlightFeelProfile::shaped(mode);
        let dwell = Duration::from_millis(u64::from(profile.horizontal.neutral.dwell_ms));
        assert!(dwell > Duration::ZERO, "{mode:?} must state a dwell");
        let shaped = ValidatedFlightFeelProfile::new(profile)
            .unwrap_or_else(|error| panic!("{mode:?} must be a valid profile: {error}"));
        let expected = *FeelDigest::calculate(&shaped).expect("digest").as_bytes();
        adapter
            .stage_control_feel(shaped)
            .unwrap_or_else(|error| panic!("{mode:?} must be stageable: {error}"));

        // A deflected stick does not activate it.
        let deflected = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
        assert_eq!(
            adapter.apply_control(&deflected).disposition,
            Disposition::Accepted
        );
        assert_eq!(
            active_digest(&adapter),
            before,
            "{mode:?} activated deflected"
        );

        // Held at centre, it arrives — and not before the dwell it states. The
        // clock is advanced rather than waited on, so the boundary is driven
        // deterministically at a rate a control loop actually reports at.
        let step = Duration::from_millis(20);
        let mut held = Duration::ZERO;
        let mut activated_after = None;
        for _ in 0..40 {
            assert_eq!(
                adapter.apply_control(&neutral_frame()).disposition,
                Disposition::Accepted
            );
            if active_digest(&adapter) == expected {
                activated_after = Some(held);
                break;
            }
            adapter.uplink_mut().expect("uplink").advance_clock(step);
            held += step;
        }

        let elapsed = activated_after
            .unwrap_or_else(|| panic!("{mode:?} never activated while held at centre"));
        assert!(
            elapsed >= dwell,
            "{mode:?} activated after {elapsed:?}, before its {dwell:?} dwell"
        );
    }
}

#[test]
fn an_operator_asks_for_a_mode_by_name_and_gets_the_qualified_one() {
    // The choice an operator makes is between three named laws, not between
    // arbitrary profiles. A caller that could supply any profile could supply
    // one nobody qualified; a mode a vehicle advertises is one somebody stood
    // behind.
    for mode in [FeelMode::Precision, FeelMode::Balanced, FeelMode::Agile] {
        let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
        fc.set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout");
        let mut adapter = airborne_adapter_with_fc(&fc);

        let staged = adapter
            .request_feel_mode(mode)
            .unwrap_or_else(|error| panic!("{mode:?} must be requestable: {error}"));
        let named = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped(mode))
            .expect("the named mode is a valid profile");
        let expected = FeelDigest::calculate(&named).expect("digest the named mode");
        assert_eq!(
            staged.as_bytes(),
            expected.as_bytes(),
            "{mode:?} staged a different law than the one it names"
        );

        // It arrives at the same neutral boundary a rollback uses, so asking
        // for a mode does not change the law under a deflected stick.
        let deflected = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
        assert_eq!(
            adapter.apply_control(&deflected).disposition,
            Disposition::Accepted
        );
        receive_frame(&fc, "deflected response frame");
        assert_ne!(
            active_digest(&adapter),
            *expected.as_bytes(),
            "{mode:?} activated under a deflected stick"
        );
    }
}

#[test]
fn a_physical_vehicle_refuses_a_mode_request() {
    // These laws are shaped for a simulator and qualified there. A physical
    // gateway takes its command law from the aircraft it is bound to, and a
    // request to change it from the ground is refused rather than negotiated.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = airborne_adapter_with_fc(&fc).with_profile(crate::AviateProfile::Physical);
    assert!(adapter.request_feel_mode(FeelMode::Agile).is_err());
}

#[test]
fn a_feel_request_on_a_control_frame_stages_the_named_law() {
    // The operator's path end to end at the vehicle: a typed action arrives on
    // a control frame, is accepted, and the named law is staged — waiting for
    // a neutral boundary rather than changing under the stick that carried it.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    fc.set_read_timeout(Some(Duration::from_secs(1)))
        .expect("read timeout");
    let mut adapter = airborne_adapter_with_fc(&fc);
    let before = active_digest(&adapter);

    // The request rides a frame whose stick is deflected, which is the case
    // that matters: an operator changes mode while flying.
    let mut frame = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
    frame.actions = vec![pilotage_protocol::ControlAction::FeelModeRequest {
        target: pilotage_protocol::FeelTarget::Agile,
    }];
    let outcome = adapter.apply_control(&frame);
    assert_eq!(outcome.disposition, Disposition::Accepted);
    receive_frame(&fc, "feel request response frame");

    let answered = outcome
        .action_results
        .iter()
        .find(|result| {
            matches!(
                result.action,
                pilotage_protocol::ControlAction::FeelModeRequest { .. }
            )
        })
        .expect("the request was answered");
    assert!(answered.accepted, "refused: {}", answered.detail);
    assert_eq!(
        active_digest(&adapter),
        before,
        "the law changed under the stick"
    );

    // Held at centre, the requested law arrives.
    let named = ValidatedFlightFeelProfile::new(FlightFeelProfile::shaped(FeelMode::Agile))
        .expect("the named mode is a valid profile");
    let expected = *FeelDigest::calculate(&named)
        .expect("digest the named mode")
        .as_bytes();
    let step = Duration::from_millis(20);
    let mut arrived = false;
    for _ in 0..40 {
        adapter
            .uplink_mut()
            .expect("uplink")
            .seed_hold_for_test([1.0, 2.0, 3.0]);
        assert_eq!(
            adapter.apply_control(&neutral_frame()).disposition,
            Disposition::Accepted
        );
        receive_frame(&fc, "neutral response frame");
        if active_digest(&adapter) == expected {
            arrived = true;
            break;
        }
        adapter.uplink_mut().expect("uplink").advance_clock(step);
    }
    assert!(arrived, "the requested law never arrived");
}

#[test]
fn the_vehicle_advertises_the_modes_it_can_serve() {
    // A client offers only what the vehicle advertises, so a control never
    // asks for a law the vehicle has no qualified profile for.
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let adapter = airborne_adapter_with_fc(&fc);
    let advertised = adapter
        .advertised_capabilities()
        .vehicles
        .into_iter()
        .flat_map(|vehicle| vehicle.scopes)
        .flat_map(|scope| scope.actions)
        .find(|action| action.action == pilotage_protocol::ActionKind::FeelModeRequest)
        .expect("the vehicle advertises a feel-mode request");
    assert_eq!(
        advertised.feel_targets,
        vec![
            pilotage_protocol::FeelTarget::Precision,
            pilotage_protocol::FeelTarget::Balanced,
            pilotage_protocol::FeelTarget::Agile,
        ]
    );
    assert!(
        advertised.mode_targets.is_empty(),
        "a feel request carries no flight-mode target"
    );
}

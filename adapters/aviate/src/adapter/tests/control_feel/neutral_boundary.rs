//! Control-feel activation neutral-boundary tests.

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_control_feel::{FeelDigest, FeelMode, FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_protocol::{ControlIntent, LogicalAxisId};

use super::*;

fn candidate_with_horizontal_dwell(id: &str, dwell_ms: u32) -> ValidatedFlightFeelProfile {
    let mut profile = FlightFeelProfile::legacy_compatibility();
    profile.profile_id = id.to_owned();
    profile.mode = FeelMode::Balanced;
    profile.horizontal.neutral.dwell_ms = dwell_ms;
    ValidatedFlightFeelProfile::new(profile).expect("valid dwell candidate")
}

fn assert_active_digest(adapter: &AviateAdapter, expected: &ValidatedFlightFeelProfile) {
    let expected = *FeelDigest::calculate(expected)
        .expect("candidate digest")
        .as_bytes();
    assert_eq!(active_digest(adapter), expected);
}

fn advance(adapter: &mut AviateAdapter, milliseconds: u64) {
    adapter
        .uplink_mut()
        .expect("uplink")
        .advance_clock(Duration::from_millis(milliseconds));
}

fn activate_after_dwell(
    adapter: &mut AviateAdapter,
    profile: &ValidatedFlightFeelProfile,
    dwell_ms: u64,
) {
    adapter
        .stage_control_feel(profile.clone())
        .expect("stage candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    advance(adapter, dwell_ms);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(adapter, profile);
}

#[test]
fn pending_profile_requires_its_complete_neutral_dwell() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    let original = active_digest(&adapter);
    let next = candidate_with_horizontal_dwell("alia250-pending-dwell", 40);
    adapter
        .stage_control_feel(next.clone())
        .expect("stage candidate");

    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), original);
    advance(&mut adapter, 39);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), original);
    advance(&mut adapter, 1);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &next);
}

#[test]
fn active_profile_requires_its_complete_neutral_dwell() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    let active = candidate_with_horizontal_dwell("alia250-active-dwell", 40);
    adapter
        .stage_control_feel(active.clone())
        .expect("stage active candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    advance(&mut adapter, 40);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &active);
    assert!(adapter.take_control_feel_change().is_some());

    let deflected = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
    assert_eq!(
        adapter.apply_control(&deflected).disposition,
        Disposition::Accepted
    );
    let next = candidate("alia250-after-active-dwell");
    adapter
        .stage_control_feel(next.clone())
        .expect("stage next candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &active);
    advance(&mut adapter, 39);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &active);
    advance(&mut adapter, 1);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &next);
}

#[test]
fn long_frame_gaps_use_the_shaper_time_cap() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    let original = active_digest(&adapter);
    let active = candidate_with_horizontal_dwell("alia250-capped-active-dwell", 200);
    adapter
        .stage_control_feel(active.clone())
        .expect("stage active candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    advance(&mut adapter, 200);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), original);
    advance(&mut adapter, 200);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &active);

    let deflected = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
    assert_eq!(
        adapter.apply_control(&deflected).disposition,
        Disposition::Accepted
    );
    let next = candidate("alia250-after-capped-active-dwell");
    adapter
        .stage_control_feel(next.clone())
        .expect("stage next candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    advance(&mut adapter, 200);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &active);
    advance(&mut adapter, 200);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &next);
}

#[test]
fn a_scope_change_restarts_the_pending_neutral_dwell() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    let original = active_digest(&adapter);
    let next = candidate_with_horizontal_dwell("alia250-scope-dwell", 40);
    adapter
        .stage_control_feel(next.clone())
        .expect("stage candidate");

    let direct_neutral = direct_frame(0.0, 0.0, 0.0, 0.5);
    assert_eq!(
        adapter.apply_control(&direct_neutral).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), original);
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    advance(&mut adapter, 40);
    assert_eq!(
        adapter.apply_control(&direct_neutral).disposition,
        Disposition::Accepted
    );
    assert_eq!(active_digest(&adapter), original);
    advance(&mut adapter, 40);
    assert_eq!(
        adapter.apply_control(&direct_neutral).disposition,
        Disposition::Accepted
    );
    assert_active_digest(&adapter, &next);
}

fn assert_nonfinite_velocity_restarts_dwell(nonfinite: f32) {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    let original = active_digest(&adapter);
    let next = candidate_with_horizontal_dwell("alia250-nonfinite-velocity", 40);
    adapter
        .stage_control_feel(next.clone())
        .expect("stage candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    advance(&mut adapter, 20);
    let mut invalid = neutral_frame();
    let Some(ControlIntent::Velocity(velocity)) = invalid.intent.as_mut() else {
        panic!("velocity intent");
    };
    velocity.vx = nonfinite;
    adapter.apply_control(&invalid);
    assert_eq!(active_digest(&adapter), original);

    advance(&mut adapter, 100);
    adapter.apply_control(&neutral_frame());
    assert_eq!(active_digest(&adapter), original);
    advance(&mut adapter, 40);
    adapter.apply_control(&neutral_frame());
    assert_active_digest(&adapter, &next);
}

#[test]
fn nonfinite_velocity_restarts_the_pending_neutral_dwell() {
    for nonfinite in [f32::NAN, f32::INFINITY] {
        assert_nonfinite_velocity_restarts_dwell(nonfinite);
    }
}

#[test]
fn nonfinite_direct_attitude_restarts_the_pending_neutral_dwell() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    let original = active_digest(&adapter);
    let next = candidate_with_horizontal_dwell("alia250-nonfinite-direct", 40);
    adapter
        .stage_control_feel(next.clone())
        .expect("stage candidate");
    let neutral = direct_frame(0.0, 0.0, 0.0, 0.5);
    assert_eq!(
        adapter.apply_control(&neutral).disposition,
        Disposition::Accepted
    );
    advance(&mut adapter, 20);
    let mut invalid = neutral.clone();
    let Some(ControlIntent::AttitudeThrust(attitude)) = invalid.intent.as_mut() else {
        panic!("attitude intent");
    };
    attitude.qx = f32::NAN;
    adapter.apply_control(&invalid);
    assert_eq!(active_digest(&adapter), original);

    advance(&mut adapter, 100);
    adapter.apply_control(&neutral);
    assert_eq!(active_digest(&adapter), original);
    advance(&mut adapter, 40);
    adapter.apply_control(&neutral);
    assert_active_digest(&adapter, &next);
}

#[test]
fn rollback_requires_fresh_current_and_prior_neutral_dwell() {
    for (prior_dwell_ms, current_dwell_ms) in [(40_u64, 80_u64), (80, 40)] {
        let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
        let mut adapter = adapter_with_fc(&fc);
        let prior = candidate_with_horizontal_dwell(
            &format!("alia250-rollback-prior-{prior_dwell_ms}"),
            prior_dwell_ms as u32,
        );
        let current = candidate_with_horizontal_dwell(
            &format!("alia250-rollback-current-{current_dwell_ms}"),
            current_dwell_ms as u32,
        );
        activate_after_dwell(&mut adapter, &prior, prior_dwell_ms);
        activate_after_dwell(&mut adapter, &current, current_dwell_ms);

        let deflected = flight_frame(vec![(LogicalAxisId::new(PITCH_AXIS), 0.5)], vec![]);
        assert_eq!(
            adapter.apply_control(&deflected).disposition,
            Disposition::Accepted
        );
        assert!(adapter.stage_control_feel_rollback());
        assert_eq!(
            adapter.apply_control(&neutral_frame()).disposition,
            Disposition::Accepted
        );
        advance(&mut adapter, 40);
        assert_eq!(
            adapter.apply_control(&neutral_frame()).disposition,
            Disposition::Accepted
        );
        assert_active_digest(&adapter, &current);
        advance(&mut adapter, 40);
        assert_eq!(
            adapter.apply_control(&neutral_frame()).disposition,
            Disposition::Accepted
        );
        assert_active_digest(&adapter, &prior);
    }
}

//! Pointing-state behavior: rate integration inside the producer's
//! travel, detent stepping at both ends, and the recenter contract.

#![allow(clippy::expect_used, clippy::panic)]

use super::{
    MAX_PITCH_RATE_RPS, MODE_FPV, MODE_GIMBAL, PAN_LIMIT_RAD, PAYLOAD_VIEW_HOLD, PointingState,
    TILT_MAX_RAD, TILT_MIN_RAD, ZOOM_DETENTS,
};

#[test]
fn a_rate_integrates_into_held_pointing() {
    let mut state = PointingState::default();
    let clamped = state.integrate(0.0, 0.6);
    assert!(!clamped, "a demand inside the envelope is not constrained");
    let command = state.command();
    assert!(command.pan_rad > 0.0, "a positive yaw rate pans right");
    assert!(
        (command.pan_rad - 0.02).abs() < 0.001,
        "one 30 Hz step of 0.6 rad/s is 0.02 rad, got {}",
        command.pan_rad
    );
    // Integration accumulates: the payload holds where it was pointed.
    state.integrate(0.0, 0.6);
    assert!(state.command().pan_rad > command.pan_rad);
}

#[test]
fn travel_limits_clamp_and_report_constrained() {
    let mut state = PointingState::default();
    // Drive tilt past its upward limit; the state stops at the limit and
    // says so rather than reporting the full demand as enacted.
    let mut constrained = false;
    for _ in 0..200 {
        constrained |= state.integrate(MAX_PITCH_RATE_RPS, 0.0);
    }
    assert!(constrained, "hitting the travel limit must report clamped");
    assert!((state.command().tilt_rad - TILT_MAX_RAD).abs() < 1e-5);

    let mut down = PointingState::default();
    for _ in 0..200 {
        down.integrate(-MAX_PITCH_RATE_RPS, 0.0);
    }
    assert!((down.command().tilt_rad - TILT_MIN_RAD).abs() < 1e-5);
}

#[test]
fn a_rate_beyond_the_envelope_is_clamped_not_applied() {
    let mut state = PointingState::default();
    let clamped = state.integrate(0.0, MAX_PITCH_RATE_RPS * 10.0);
    assert!(clamped);
    let expected = MAX_PITCH_RATE_RPS / 30.0;
    assert!(
        (state.command().pan_rad - expected).abs() < 1e-4,
        "the enacted rate is the advertised envelope, got {}",
        state.command().pan_rad
    );
}

#[test]
fn a_non_finite_rate_holds_still() {
    let mut state = PointingState::default();
    assert!(state.integrate(f32::NAN, f32::INFINITY));
    assert_eq!(state.command().pan_rad, 0.0);
    assert_eq!(state.command().tilt_rad, 0.0);
}

#[test]
fn recenter_stows_the_pointing_and_keeps_the_detent() {
    let mut state = PointingState::default();
    state.integrate(0.5, 0.5);
    assert!(state.zoom_in());
    let detent_before = state.command().zoom_detent;
    state.recenter();
    let command = state.command();
    assert_eq!(command.pan_rad, 0.0);
    assert_eq!(command.tilt_rad, 0.0);
    assert_eq!(
        command.zoom_detent, detent_before,
        "recentering aims the payload; it does not change the camera model"
    );
}

#[test]
fn detents_step_and_refuse_past_the_ends() {
    let mut state = PointingState::default();
    assert!(!state.zoom_out(), "already at the widest detent");
    for step in 1..ZOOM_DETENTS.len() {
        assert!(state.zoom_in(), "step {step} must be available");
        assert_eq!(state.command().zoom_detent as usize, step);
    }
    assert!(!state.zoom_in(), "already at the narrowest detent");
    assert!(state.zoom_out());
    assert_eq!(state.command().zoom_detent as usize, ZOOM_DETENTS.len() - 2);
}

#[test]
fn each_detent_publishes_its_own_calibration() {
    // ADR-0021: a zoomed frame must carry the camera model it was
    // captured with, so no two detents may share a calibration.
    let mut state = PointingState::default();
    let mut seen = vec![state.detent().calibration_id];
    while state.zoom_in() {
        let id = state.detent().calibration_id;
        assert!(!seen.contains(&id), "detent calibrations must be distinct");
        seen.push(id);
    }
    assert_eq!(seen.len(), ZOOM_DETENTS.len());
    assert!(seen.iter().all(|id| *id != 0), "NONE is not a calibration");
}

#[test]
fn pan_wraps_nowhere_and_stays_inside_travel() {
    let mut state = PointingState::default();
    for _ in 0..1000 {
        state.integrate(0.0, 0.8);
    }
    assert!((state.command().pan_rad - PAN_LIMIT_RAD).abs() < 1e-4);
}

#[test]
fn the_detent_table_matches_the_producer_s_own() {
    // The adapter steps detents and the producer enacts them. If the two
    // tables disagree, a frame carries a field of view — and a
    // calibration — it was not captured with. Read the producer's header
    // and pin both tables against each other.
    let header = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../sim/xplane/camera/camera_state.h");
    let Ok(source) = std::fs::read_to_string(&header) else {
        panic!("the producer's detent table must be readable at {header:?}");
    };
    let table = source
        .split("constexpr ZoomDetent kZoomDetents[] = {")
        .nth(1)
        .and_then(|rest| rest.split("};").next())
        .expect("the producer declares a detent table");
    let producer: Vec<(f32, u32)> = table
        .lines()
        .filter_map(|line| {
            let entry = line.trim().trim_start_matches('{').trim_end_matches("},");
            let (fov, calibration) = entry.split_once(',')?;
            let fov = fov.trim().trim_end_matches('F').parse().ok()?;
            let calibration = calibration
                .trim()
                .trim_end_matches('U')
                .replace('\'', "")
                .strip_prefix("0x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())?;
            Some((fov, calibration))
        })
        .collect();

    assert_eq!(
        producer.len(),
        ZOOM_DETENTS.len(),
        "the two detent ladders must have the same number of steps"
    );
    for (index, (fov, calibration)) in producer.iter().enumerate() {
        assert!(
            (ZOOM_DETENTS[index].field_of_view_deg - fov).abs() < f32::EPSILON,
            "detent {index} field of view disagrees: adapter {}, producer {fov}",
            ZOOM_DETENTS[index].field_of_view_deg
        );
        assert_eq!(
            ZOOM_DETENTS[index].calibration_id, *calibration,
            "detent {index} calibration id disagrees"
        );
    }
}

#[test]
fn the_view_starts_forward_and_follows_the_aim() {
    // One rendered view: showing the payload means NOT showing the
    // vehicle's forward camera, so the forward view is the resting
    // state and aiming is what selects the payload.
    let mut state = PointingState::default();
    assert_eq!(state.command().mode, 1, "the forward view is the default");
    assert!(
        state.view_is_stale(),
        "a producer outlives a session, so the view is stated, not assumed"
    );
    state.note_published();
    assert!(!state.view_is_stale(), "nothing to republish once stated");

    state.aim();
    assert_eq!(state.command().mode, 2, "aiming selects the payload view");
    assert!(state.view_is_stale(), "the producer must be told");
    state.note_published();
    assert!(!state.view_is_stale());
}

#[test]
fn the_view_returns_forward_after_aiming_stops() {
    let mut state = PointingState::default();
    state.aim();
    state.note_published();
    // Age the aim past the hold without sleeping.
    state.aimed_at = Some(std::time::Instant::now() - super::PAYLOAD_VIEW_HOLD);
    assert_eq!(
        state.command().mode,
        1,
        "a released quasimode returns the forward feed"
    );
    assert!(
        state.view_is_stale(),
        "the producer still renders the payload until it is told otherwise"
    );
}

#[test]
fn a_held_lease_is_not_an_aim() {
    use pilotage_protocol::{ControlAction, ControlIntent, GimbalRateIntent};

    use super::demands_payload_view;

    // A client streams every scope it holds, so a neutral frame arrives
    // ~30 times a second for as long as the operator holds the lease.
    // Reading that as aiming would select the payload view once and
    // never release it, leaving the forward feed dark for the session.
    let neutral = GimbalRateIntent {
        pitch_rate: 0.0,
        yaw_rate: 0.0,
    };
    assert!(!demands_payload_view(
        &[],
        Some(ControlIntent::GimbalRate(neutral))
    ));
    assert!(!demands_payload_view(&[], None));

    let moving = GimbalRateIntent {
        pitch_rate: 0.0,
        yaw_rate: -0.4,
    };
    assert!(demands_payload_view(
        &[],
        Some(ControlIntent::GimbalRate(moving))
    ));
    // A discrete press is a demand even with the stick centered:
    // recentering or stepping a detent is something to look at.
    assert!(demands_payload_view(
        &[ControlAction::GimbalRecenter],
        Some(ControlIntent::GimbalRate(neutral))
    ));
    assert!(demands_payload_view(&[ControlAction::CameraZoomIn], None));
}

#[test]
fn a_neutral_stream_sustains_an_aimed_view_but_never_starts_one() {
    let mut pointing = PointingState::default();
    // A held lease is not an aim: neutral sustain on a never-aimed
    // pointing selects nothing.
    pointing.sustain_aim();
    assert_eq!(pointing.mode(), MODE_FPV);
    // An aim selects the payload view, and the held scope's neutral
    // liveness stream keeps it selected past the hold window.
    pointing.aim();
    assert_eq!(pointing.mode(), MODE_GIMBAL);
    pointing.sustain_aim();
    assert_eq!(pointing.mode(), MODE_GIMBAL);
}

#[test]
fn an_expired_view_is_not_resurrected_by_neutral_frames() {
    let mut pointing = PointingState::default();
    pointing.aim();
    // Simulate the stream stopping past the hold window by aging the
    // stamp directly: sustain must NOT re-arm an expired view.
    pointing.aimed_at = Some(std::time::Instant::now() - PAYLOAD_VIEW_HOLD * 2);
    assert_eq!(pointing.mode(), MODE_FPV);
    pointing.sustain_aim();
    assert_eq!(pointing.mode(), MODE_FPV);
}

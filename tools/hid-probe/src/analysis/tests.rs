#![allow(clippy::expect_used, clippy::panic)]

use pilotage_input::{
    CalibrationCandidate, CenterBehavior, CharacterizationCapture, CharacterizationError,
    DeadzoneEvidenceMethod, DeadzoneEvidenceStatus, PromotionConfirmation, SamplingSource,
    TimestampSource, canonical_candidate_digest, normalize_axis, parse_profile_bytes,
    promote_calibration_candidate,
};

use super::characterize;
use crate::error::ProbeError;

const CAPTURE_BYTES: &[u8] = include_bytes!("../../fixtures/synthetic-capture.json");
const PROFILE_BYTES: &[u8] = include_bytes!("../../fixtures/synthetic-profile.json");

fn capture() -> CharacterizationCapture {
    serde_json::from_slice(CAPTURE_BYTES).expect("synthetic capture")
}

fn candidate() -> CalibrationCandidate {
    characterize(CAPTURE_BYTES, &capture(), PROFILE_BYTES).expect("characterize")
}

fn axis<'a>(
    candidate: &'a CalibrationCandidate,
    logical: &str,
) -> &'a pilotage_input::AxisCharacterization {
    candidate
        .axes
        .iter()
        .find(|axis| axis.logical == logical)
        .expect("candidate axis")
}

#[test]
fn synthetic_fixture_recovers_center_range_noise_drift_and_timing() {
    let candidate = candidate();
    assert_eq!(candidate.sample_count, 40);
    assert!((candidate.timing.median_period_us - 4_000.0).abs() < 0.1);
    assert!(candidate.timing.jitter_mad_us < 0.1);
    assert_eq!(candidate.timing.dropped_report_count, 1);
    assert!(candidate.confidence >= 0.8);
    assert_eq!(candidate.source_capture_digest.len(), 64);

    let roll = axis(&candidate, "roll");
    assert_eq!(roll.source_index, 0);
    assert!(!roll.invert);
    assert!((roll.observed_center - 1000.0).abs() <= 1.0);
    assert_eq!((roll.observed_min, roll.observed_max), (0.0, 2000.0));
    assert!(roll.center_noise > 0.1 && roll.center_noise < 3.0);
    assert_eq!(roll.center_behavior, CenterBehavior::Stable);

    let pitch = axis(&candidate, "pitch");
    assert_eq!(pitch.source_index, 1);
    assert!(pitch.invert);
    assert!((pitch.observed_center - 514.5).abs() <= 1.0);
    assert_eq!((pitch.observed_min, pitch.observed_max), (0.0, 1000.0));
    assert!((pitch.center_drift_per_second - 450.0).abs() < 100.0);
    assert_eq!(pitch.center_behavior, CenterBehavior::Drifting);
}

#[test]
fn held_center_and_slow_drift_have_different_results() {
    let candidate = candidate();
    assert_eq!(
        axis(&candidate, "roll").center_behavior,
        CenterBehavior::Stable
    );
    assert_eq!(
        axis(&candidate, "pitch").center_behavior,
        CenterBehavior::Drifting
    );
}

#[test]
fn cross_axis_motion_is_rejected() {
    let mut capture = capture();
    for sample in capture
        .samples
        .iter_mut()
        .filter(|sample| (16..=27).contains(&sample.sequence))
    {
        sample.axes[1] = sample.axes[0];
    }
    let bytes = serde_json::to_vec(&capture).expect("capture bytes");
    assert!(matches!(
        characterize(&bytes, &capture, PROFILE_BYTES),
        Err(ProbeError::AmbiguousMovement { logical, .. }) if logical == "roll"
    ));
}

#[test]
fn a_centered_axis_without_both_sides_of_travel_is_rejected() {
    let mut capture = capture();
    for sample in capture
        .samples
        .iter_mut()
        .filter(|sample| (22..=25).contains(&sample.sequence))
    {
        sample.axes[0] = 1000.0;
    }
    let bytes = serde_json::to_vec(&capture).expect("capture bytes");
    assert!(matches!(
        characterize(&bytes, &capture, PROFILE_BYTES),
        Err(ProbeError::IncompleteMovement {
            logical,
            source_index: 0,
        }) if logical == "roll"
    ));
}

#[test]
fn two_named_movements_cannot_select_one_source_axis() {
    let mut capture = capture();
    for sample in capture
        .samples
        .iter_mut()
        .filter(|sample| (28..=39).contains(&sample.sequence))
    {
        sample.axes[0] = (sample.axes[1] - 514.5) * 2.0 + 1000.0;
        sample.axes[1] = 514.5;
    }
    let bytes = serde_json::to_vec(&capture).expect("capture bytes");
    assert!(matches!(
        characterize(&bytes, &capture, PROFILE_BYTES),
        Err(ProbeError::DuplicateMovement {
            source_index: 0,
            ..
        })
    ));
}

#[test]
fn an_observed_platform_deadzone_never_adds_a_second_deadzone() {
    let mut capture = capture();
    capture.deadzone_evidence.status = DeadzoneEvidenceStatus::Observed;
    capture.deadzone_evidence.method = DeadzoneEvidenceMethod::PairedNativeAndPlatform;
    let bytes = serde_json::to_vec(&capture).expect("capture bytes");
    let candidate = characterize(&bytes, &capture, PROFILE_BYTES).expect("characterize");
    assert!(
        candidate
            .axes
            .iter()
            .all(|axis| axis.proposed_deadzone == 0.0)
    );

    let mut tampered = candidate;
    tampered.axes[0].proposed_deadzone = 0.1;
    let confirmation = confirmation(&tampered);
    assert!(promote_calibration_candidate(PROFILE_BYTES, &tampered, &confirmation).is_err());
}

#[test]
fn promotion_requires_the_reviewed_digests_and_changes_only_device_correction() {
    let candidate = candidate();
    let mut wrong = confirmation(&candidate);
    wrong.source_capture_digest = "0".repeat(64);
    assert!(promote_calibration_candidate(PROFILE_BYTES, &candidate, &wrong).is_err());

    let baseline = parse_profile_bytes(PROFILE_BYTES).expect("baseline");
    let confirmation = confirmation(&candidate);
    let promoted = promote_calibration_candidate(PROFILE_BYTES, &candidate, &confirmation)
        .expect("confirmed promotion");
    assert_eq!(promoted.revision, baseline.revision.wrapping_add(1));
    assert_eq!(promoted.description, baseline.description);
    assert_eq!(promoted.buttons, baseline.buttons);
    assert_eq!(promoted.keys, baseline.keys);
    for before in &baseline.axes {
        let after = promoted
            .axes
            .iter()
            .find(|axis| axis.logical == before.logical)
            .expect("promoted axis");
        assert_eq!(after.expo, before.expo, "feel curve must not change");
    }
    let roll = promoted
        .axes
        .iter()
        .find(|axis| axis.logical == "roll")
        .expect("roll");
    assert_eq!(normalize_axis(roll.calibration.max, roll).value, 1.0);
    assert_eq!(normalize_axis(roll.calibration.center, roll).value, 0.0);
    assert_eq!(normalize_axis(roll.calibration.min, roll).value, -1.0);
}

#[test]
fn a_candidate_change_after_review_is_rejected() {
    let reviewed = candidate();
    let confirmation = confirmation(&reviewed);
    let mut changed = reviewed;
    changed.axes[0].invert = !changed.axes[0].invert;
    assert!(matches!(
        promote_calibration_candidate(PROFILE_BYTES, &changed, &confirmation),
        Err(CharacterizationError::CandidateConfirmationMismatch { .. })
    ));
}

#[test]
fn promotion_rejects_a_crafted_one_sided_centered_axis() {
    let mut candidate = candidate();
    let roll = candidate
        .axes
        .iter_mut()
        .find(|axis| axis.logical == "roll")
        .expect("roll");
    roll.observed_min = roll.observed_center;
    roll.calibration.min = roll.observed_center - 0.001;
    let confirmation = confirmation(&candidate);
    assert!(matches!(
        promote_calibration_candidate(PROFILE_BYTES, &candidate, &confirmation),
        Err(CharacterizationError::InvalidAxisEvidence { logical }) if logical == "roll"
    ));
}

#[test]
fn candidate_schema_rejects_feel_curve_and_vehicle_limit_fields() {
    let mut value = serde_json::to_value(candidate()).expect("candidate value");
    let object = value.as_object_mut().expect("candidate object");
    object.insert("feel_curve".to_owned(), serde_json::json!({ "expo": 0.5 }));
    object.insert(
        "vehicle_limit".to_owned(),
        serde_json::json!({ "speed": 10 }),
    );
    assert!(serde_json::from_value::<CalibrationCandidate>(value).is_err());
}

#[test]
fn paired_native_and_browser_candidates_normalize_the_same_physical_positions() {
    let native_candidate = candidate();
    let native_profile = promote(&native_candidate);
    let mut browser_capture = capture();
    browser_capture.source = SamplingSource::BrowserGamepad;
    browser_capture.timestamp_source = TimestampSource::Source;
    browser_capture.deadzone_evidence.status = DeadzoneEvidenceStatus::NotObserved;
    browser_capture.deadzone_evidence.method = DeadzoneEvidenceMethod::PairedNativeAndPlatform;
    for sample in &mut browser_capture.samples {
        sample.source_at_us = Some(sample.observed_at_us);
        sample.axes[0] = unit_value(sample.axes[0], 0.0, 1000.0, 2000.0);
        sample.axes[1] = unit_value(sample.axes[1], 0.0, 514.5, 1000.0);
    }
    let browser_bytes = serde_json::to_vec(&browser_capture).expect("browser capture bytes");
    let browser_candidate = characterize(&browser_bytes, &browser_capture, PROFILE_BYTES)
        .expect("browser characterize");
    let browser_profile = promote(&browser_candidate);

    for (native_raw, browser_raw) in [
        (0.0, -1.0),
        (250.0, -0.75),
        (1000.0, 0.0),
        (1500.0, 0.5),
        (2000.0, 1.0),
    ] {
        assert_paths_match(
            &native_profile,
            &browser_profile,
            "roll",
            native_raw,
            browser_raw,
        );
    }
    for native_raw in [0.0, 250.0, 514.5, 750.0, 1000.0] {
        let browser_raw = unit_value(native_raw, 0.0, 514.5, 1000.0);
        assert_paths_match(
            &native_profile,
            &browser_profile,
            "pitch",
            native_raw,
            browser_raw,
        );
    }
}

#[test]
fn source_clock_deadzone_golden_ignores_arrival_queue_delay() {
    let reference_capture = browser_source_capture();
    let reference_bytes = serde_json::to_vec(&reference_capture).expect("reference bytes");
    let reference = characterize(&reference_bytes, &reference_capture, PROFILE_BYTES)
        .expect("reference characterize");
    let mut delayed_capture = reference_capture;
    for sample in &mut delayed_capture.samples {
        sample.observed_at_us = sample.sequence * 1_000_000;
    }
    let delayed_bytes = serde_json::to_vec(&delayed_capture).expect("delayed bytes");
    let delayed = characterize(&delayed_bytes, &delayed_capture, PROFILE_BYTES)
        .expect("delayed characterize");

    assert_eq!(reference.timing, delayed.timing);
    for logical in ["roll", "pitch"] {
        assert_eq!(
            axis(&reference, logical).proposed_deadzone,
            axis(&delayed, logical).proposed_deadzone,
            "{logical} dead zone must use the selected source clock"
        );
    }
}

fn browser_source_capture() -> CharacterizationCapture {
    let mut capture = capture();
    capture.source = SamplingSource::BrowserGamepad;
    capture.timestamp_source = TimestampSource::Source;
    capture.deadzone_evidence.status = DeadzoneEvidenceStatus::NotObserved;
    capture.deadzone_evidence.method = DeadzoneEvidenceMethod::PairedNativeAndPlatform;
    for sample in &mut capture.samples {
        sample.source_at_us = Some(sample.observed_at_us);
    }
    capture
}

fn promote(candidate: &CalibrationCandidate) -> pilotage_input::DeviceProfile {
    let confirmation = confirmation(candidate);
    promote_calibration_candidate(PROFILE_BYTES, candidate, &confirmation).expect("promotion")
}

fn confirmation(candidate: &CalibrationCandidate) -> PromotionConfirmation {
    PromotionConfirmation {
        source_capture_digest: candidate.source_capture_digest.clone(),
        candidate_digest: canonical_candidate_digest(candidate).expect("candidate digest"),
    }
}

fn unit_value(raw: f32, minimum: f32, center: f32, maximum: f32) -> f32 {
    if raw >= center {
        (raw - center) / (maximum - center)
    } else {
        (raw - center) / (center - minimum)
    }
}

fn assert_paths_match(
    native: &pilotage_input::DeviceProfile,
    browser: &pilotage_input::DeviceProfile,
    logical: &str,
    native_raw: f32,
    browser_raw: f32,
) {
    let native_axis = native
        .axes
        .iter()
        .find(|axis| axis.logical == logical)
        .expect("native axis");
    let browser_axis = browser
        .axes
        .iter()
        .find(|axis| axis.logical == logical)
        .expect("browser axis");
    let native_value = normalize_axis(native_raw, native_axis).value;
    let browser_value = normalize_axis(browser_raw, browser_axis).value;
    assert!(
        (native_value - browser_value).abs() < 0.002,
        "{logical}: native {native_value} browser {browser_value}"
    );
}

#![allow(clippy::expect_used, clippy::panic)]

use super::analysis::validate_capture_byte_count;
use crate::{
    AnalysisError, AxisCharacterization, AxisConfig, CalibrationCandidate, CenterBehavior,
    CharacterizationCapture, CharacterizationError, DeadzoneEvidenceMethod, DeadzoneEvidenceStatus,
    NeutralPosition, PromotionConfirmation, SamplingSource, SourceAxisContract, TimestampSource,
    TimingObservation, canonical_candidate_digest, characterize_capture, content_digest,
    normalize_axis, parse_profile_bytes, promote_calibration_candidate,
};

#[path = "tests/interoperability.rs"]
mod interoperability;
#[path = "tests/validation.rs"]
mod validation;

const CONTRACT_BYTES: &[u8] =
    include_bytes!("../../../../tools/hid-probe/fixtures/synthetic-source-contract.json");
const CAPTURE_BYTES: &[u8] =
    include_bytes!("../../../../tools/hid-probe/fixtures/synthetic-capture.json");
const PROFILE_BYTES: &[u8] =
    include_bytes!("../../../../tools/hid-probe/fixtures/synthetic-profile.json");
const BROWSER_CONTRACT_BYTES: &[u8] =
    include_bytes!("../../../../tools/hid-probe/fixtures/browser-source-contract.json");
const BROWSER_CAPTURE_BYTES: &[u8] =
    include_bytes!("../../../../tools/hid-probe/fixtures/browser-capture.json");
const BROWSER_PROFILE_BYTES: &[u8] =
    include_bytes!("../../../../tools/hid-probe/fixtures/browser-profile.json");

fn capture() -> CharacterizationCapture {
    serde_json::from_slice(CAPTURE_BYTES).expect("synthetic capture")
}

fn candidate() -> CalibrationCandidate {
    characterize_capture(CONTRACT_BYTES, CAPTURE_BYTES, PROFILE_BYTES).expect("characterize")
}

fn browser_candidate() -> CalibrationCandidate {
    characterize_capture(
        BROWSER_CONTRACT_BYTES,
        BROWSER_CAPTURE_BYTES,
        BROWSER_PROFILE_BYTES,
    )
    .expect("browser candidate")
}

fn axis<'a>(candidate: &'a CalibrationCandidate, logical: &str) -> &'a AxisCharacterization {
    candidate
        .axes
        .iter()
        .find(|axis| axis.logical == logical)
        .expect("candidate axis")
}

#[test]
fn synthetic_fixture_recovers_center_range_noise_drift_and_timing() {
    let candidate = candidate();
    assert_eq!(candidate.source, SamplingSource::Synthetic);
    assert_eq!(candidate.sample_count, 40);
    assert_eq!(
        candidate.timing.observation,
        TimingObservation::InjectedSamples
    );
    assert!((candidate.timing.median_period_us - 4_000.0).abs() < 0.1);
    assert!(candidate.timing.jitter_mad_us < 0.1);
    assert_eq!(candidate.timing.dropped_report_count, None);
    assert!(candidate.confidence >= 0.8);
    assert_eq!(candidate.source_capture_digest.len(), 64);
    assert_eq!(candidate.source_contract_digest, digest_hex(CONTRACT_BYTES));

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
fn normalized_cross_axis_motion_is_rejected() {
    let mut capture = capture();
    for sample in capture
        .samples
        .iter_mut()
        .filter(|sample| (16..=27).contains(&sample.sequence))
    {
        sample.axes[1] = (sample.axes[0] / 2.0).clamp(0.0, 1000.0);
    }
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::AmbiguousMovement { logical, .. }) if logical == "roll"
    ));
}

#[test]
fn a_small_balanced_movement_cannot_become_full_scale() {
    let mut capture = capture();
    for sample in capture
        .samples
        .iter_mut()
        .filter(|sample| (16..=27).contains(&sample.sequence))
    {
        sample.axes[0] = 1000.0 + (sample.axes[0] - 1000.0) * 0.1;
    }
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::IncompleteMovement {
            logical,
            source_index: 0,
        }) if logical == "roll"
    ));
}

#[test]
fn a_centered_axis_without_both_endpoints_is_rejected() {
    let mut capture = capture();
    for sample in capture
        .samples
        .iter_mut()
        .filter(|sample| (22..=25).contains(&sample.sequence))
    {
        sample.axes[0] = 1000.0;
    }
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::IncompleteMovement {
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
        sample.axes[0] = sample.axes[1] * 2.0;
        sample.axes[1] = 514.5;
    }
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::DuplicateMovement {
            source_index: 0,
            ..
        })
    ));
}

#[test]
fn segment_prompt_gaps_do_not_become_dropped_reports() {
    let reference = candidate();
    let mut delayed = capture();
    for sample in &mut delayed.samples {
        if sample.sequence >= 16 {
            sample.observed_at_us = sample.observed_at_us.saturating_add(10_000_000);
        }
        if sample.sequence >= 28 {
            sample.observed_at_us = sample.observed_at_us.saturating_add(10_000_000);
        }
    }
    let delayed = characterize(&delayed).expect("delayed capture");
    assert_eq!(reference.timing, delayed.timing);
}

#[test]
fn one_sided_deadzone_uses_the_reachable_side() {
    let mut axis = axis(&candidate(), "roll").clone();
    axis.source_range.neutral_position = NeutralPosition::Minimum;
    axis.observed_min = 0.0;
    axis.observed_center = 0.0;
    axis.observed_max = 2000.0;
    axis.center_noise = 2.0;
    axis.center_drift_per_second = 0.0;
    axis.idle_duration_us = 1_000_000;
    assert!((axis.derived_deadzone(DeadzoneEvidenceStatus::NotObserved) - 0.004).abs() < 0.0001);
}

#[test]
fn one_sided_endpoint_noise_cannot_become_a_full_reverse_command() {
    let mut minimum = axis(&candidate(), "roll").clone();
    minimum.source_range.neutral_position = NeutralPosition::Minimum;
    minimum.observed_min = 0.0;
    minimum.observed_center = 1.0;
    minimum.observed_max = 2000.0;
    minimum.center_noise = 2.0;
    let minimum_config = one_sided_config(&minimum);
    assert_eq!(normalize_axis(0.0, &minimum_config).value, 0.0);
    assert_eq!(normalize_axis(2000.0, &minimum_config).value, 1.0);

    let mut maximum = minimum;
    maximum.source_range.neutral_position = NeutralPosition::Maximum;
    maximum.observed_min = 0.0;
    maximum.observed_center = 1999.0;
    maximum.observed_max = 2000.0;
    let maximum_config = one_sided_config(&maximum);
    assert_eq!(normalize_axis(2000.0, &maximum_config).value, 0.0);
    assert_eq!(normalize_axis(0.0, &maximum_config).value, -1.0);
}

#[test]
fn an_unmeasured_platform_deadzone_cannot_add_suppression() {
    let mut capture = capture();
    capture.source = SamplingSource::BrowserGamepad;
    capture.timestamp_source = TimestampSource::Source;
    capture.timing_observation = TimingObservation::PolledStateUpdates;
    capture.deadzone_evidence.status = DeadzoneEvidenceStatus::Unknown;
    capture.deadzone_evidence.method = DeadzoneEvidenceMethod::Unmeasured;
    capture.deadzone_evidence.sample_count = 0;
    for sample in &mut capture.samples {
        sample.source_at_us = Some(sample.observed_at_us);
    }
    let candidate = characterize(&capture).expect("browser-shaped capture");
    assert_eq!(candidate.timing.dropped_report_count, None);
    assert!(
        candidate
            .axes
            .iter()
            .all(|axis| axis.proposed_deadzone == 0.0)
    );
}

#[test]
fn promotion_recomputes_exact_evidence_and_preserves_feel() {
    let candidate = browser_candidate();
    let baseline = parse_profile_bytes(BROWSER_PROFILE_BYTES).expect("baseline");
    let promoted = promote_browser(&candidate).expect("confirmed promotion");
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
        .find(|axis| axis.logical == "slot0")
        .expect("roll");
    assert_eq!(normalize_axis(roll.calibration.max, roll).value, 1.0);
    assert_eq!(normalize_axis(roll.calibration.center, roll).value, 0.0);
    assert_eq!(normalize_axis(roll.calibration.min, roll).value, -1.0);
}

#[test]
fn a_digest_confirmed_but_changed_candidate_is_rejected() {
    let mut changed = browser_candidate();
    changed.axes[0].calibration.max = 100.0;
    let result = promote_browser(&changed);
    assert!(matches!(
        result,
        Err(CharacterizationError::CandidateEvidenceMismatch { .. })
    ));
}

#[test]
fn signed_zero_cannot_change_exact_candidate_evidence() {
    let mut changed = browser_candidate();
    assert_eq!(changed.timing.jitter_mad_us.to_bits(), 0.0f64.to_bits());
    let original_digest = canonical_candidate_digest(&changed).expect("original digest");
    changed.timing.jitter_mad_us = -0.0;
    assert_ne!(
        canonical_candidate_digest(&changed).expect("changed digest"),
        original_digest
    );
    assert!(matches!(
        promote_browser(&changed),
        Err(CharacterizationError::CandidateEvidenceMismatch { .. })
    ));
}

#[test]
fn a_changed_source_contract_is_rejected() {
    let mut contract: SourceAxisContract =
        serde_json::from_slice(CONTRACT_BYTES).expect("contract");
    contract.axes[0].maximum = 100.0;
    let contract_bytes = serde_json::to_vec(&contract).expect("contract bytes");
    assert!(matches!(
        characterize_capture(&contract_bytes, CAPTURE_BYTES, PROFILE_BYTES),
        Err(AnalysisError::ContractDigestMismatch { .. })
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
fn candidate_schema_rejects_nested_unknown_fields() {
    let mut value = serde_json::to_value(candidate()).expect("candidate value");
    value["axes"][0]["calibration"]["vehicle_limit"] = serde_json::json!(10);
    assert!(serde_json::from_value::<CalibrationCandidate>(value).is_err());

    let mut value = serde_json::to_value(candidate()).expect("candidate value");
    value["device"]["serial_number"] = serde_json::json!("not-in-schema");
    assert!(serde_json::from_value::<CalibrationCandidate>(value).is_err());
}

#[test]
fn capture_bytes_are_refused_before_decode_at_the_shared_limit() {
    assert!(validate_capture_byte_count(crate::MAX_CHARACTERIZATION_CAPTURE_BYTES).is_ok());
    assert!(matches!(
        validate_capture_byte_count(crate::MAX_CHARACTERIZATION_CAPTURE_BYTES.saturating_add(1)),
        Err(AnalysisError::CaptureTooLarge { .. })
    ));
}

#[test]
fn baseline_nested_unknowns_cannot_disappear_during_promotion() {
    let candidate = browser_candidate();
    let confirmation = PromotionConfirmation {
        source_capture_digest: candidate.source_capture_digest.clone(),
        candidate_digest: canonical_candidate_digest(&candidate).expect("candidate digest"),
    };
    let mut values = Vec::new();
    let mut axis: serde_json::Value =
        serde_json::from_slice(BROWSER_PROFILE_BYTES).expect("baseline value");
    axis["axes"][0]["vehicle_limit"] = serde_json::json!(10);
    values.push(axis);
    let mut button: serde_json::Value =
        serde_json::from_slice(BROWSER_PROFILE_BYTES).expect("baseline value");
    button["buttons"] = serde_json::json!([{
        "source_index": 0,
        "logical": "button0",
        "vehicle_limit": 10
    }]);
    values.push(button);

    for value in values {
        let bytes = serde_json::to_vec(&value).expect("baseline bytes");
        assert!(matches!(
            promote_calibration_candidate(
                BROWSER_CONTRACT_BYTES,
                BROWSER_CAPTURE_BYTES,
                &bytes,
                &candidate,
                &confirmation,
            ),
            Err(CharacterizationError::Analysis {
                source: AnalysisError::Profile { .. }
            })
        ));
    }
}

fn characterize(capture: &CharacterizationCapture) -> Result<CalibrationCandidate, AnalysisError> {
    let bytes = serde_json::to_vec(capture).expect("capture bytes");
    characterize_capture(CONTRACT_BYTES, &bytes, PROFILE_BYTES)
}

fn one_sided_config(axis: &AxisCharacterization) -> AxisConfig {
    AxisConfig {
        source_index: axis.source_index,
        logical: axis.logical.clone(),
        invert: false,
        deadzone: axis.derived_deadzone(DeadzoneEvidenceStatus::NotObserved),
        expo: 0.0,
        calibration: axis.derived_calibration(),
    }
}

fn promote_browser(
    candidate: &CalibrationCandidate,
) -> Result<crate::DeviceProfile, CharacterizationError> {
    let confirmation = PromotionConfirmation {
        source_capture_digest: candidate.source_capture_digest.clone(),
        candidate_digest: canonical_candidate_digest(candidate).expect("candidate digest"),
    };
    promote_calibration_candidate(
        BROWSER_CONTRACT_BYTES,
        BROWSER_CAPTURE_BYTES,
        BROWSER_PROFILE_BYTES,
        candidate,
        &confirmation,
    )
}

fn digest_hex(bytes: &[u8]) -> String {
    content_digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

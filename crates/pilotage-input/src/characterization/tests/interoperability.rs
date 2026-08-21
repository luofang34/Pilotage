use super::{
    BROWSER_CAPTURE_BYTES, BROWSER_CONTRACT_BYTES, BROWSER_PROFILE_BYTES, CONTRACT_BYTES,
    PROFILE_BYTES, capture, digest_hex,
};
use crate::{
    AxisCharacterization, AxisConfig, CalibrationCandidate, CharacterizationError, DeviceProfile,
    PromotionConfirmation, SamplingSource, TimingObservation, canonical_candidate_digest,
    characterize_capture, normalize_axis, promote_calibration_candidate,
};

const APPLE_CAPTURE_BYTES: &[u8] =
    include_bytes!("../../../../../tools/hid-probe/fixtures/apple-capture.json");
const WEB_GOLDEN_BYTES: &[u8] =
    include_bytes!("../../../../../clients/web-control/golden-vectors.json");

#[test]
fn apple_fixture_and_browser_evidence_produce_one_equivalent_mapping() {
    let apple = characterize_capture(CONTRACT_BYTES, APPLE_CAPTURE_BYTES, PROFILE_BYTES)
        .expect("Apple candidate");
    let browser = characterize_capture(
        BROWSER_CONTRACT_BYTES,
        BROWSER_CAPTURE_BYTES,
        BROWSER_PROFILE_BYTES,
    )
    .expect("browser candidate");
    assert_eq!(
        digest_hex(APPLE_CAPTURE_BYTES),
        "cd32524e026129d812e762290c4219ae19e1797fd4505cc0253fc821781cd415"
    );
    assert_eq!(
        canonical_candidate_digest(&apple).expect("Apple digest"),
        "f56eab8f55f5be7da308661485ff4c1f70417244486ed283251b51406ac494f3"
    );
    assert_eq!(
        digest_hex(BROWSER_CAPTURE_BYTES),
        "69ccd7c08881dad88244a5bb5eac28a68ef65fd1d2a7f4a8128e2cd616f756b1"
    );
    assert_eq!(
        canonical_candidate_digest(&browser).expect("browser digest"),
        "0900f90515634d8600c408635d6b75bf82ace948ed9ff86999543683c3aae9c9"
    );

    assert_eq!(apple.source, SamplingSource::Synthetic);
    assert_eq!(apple.timing.observation, TimingObservation::InjectedSamples);
    assert_eq!(apple.timing.dropped_report_count, None);
    assert_synthetic_promotion_is_refused(&apple);
    assert_web_golden_uses_complete_browser_trace();
    let browser_profile = promote_exact(
        BROWSER_CONTRACT_BYTES,
        BROWSER_CAPTURE_BYTES,
        BROWSER_PROFILE_BYTES,
        &browser,
    );
    assert_candidate_physical_trace_matches(&apple, &browser);
    assert_eq!(
        browser.timing.observation,
        TimingObservation::PolledStateUpdates
    );
    assert_eq!(browser.timing.dropped_report_count, None);
    assert_eq!(profile_from_web_golden(), browser_profile);
}

fn assert_synthetic_promotion_is_refused(candidate: &CalibrationCandidate) {
    let confirmation = PromotionConfirmation {
        source_capture_digest: candidate.source_capture_digest.clone(),
        candidate_digest: canonical_candidate_digest(candidate).expect("Apple digest"),
    };
    assert!(matches!(
        promote_calibration_candidate(
            CONTRACT_BYTES,
            APPLE_CAPTURE_BYTES,
            PROFILE_BYTES,
            candidate,
            &confirmation,
        ),
        Err(CharacterizationError::UnsupportedPromotionSource {
            sampling_source: SamplingSource::Synthetic
        })
    ));
}

fn promote_exact(
    contract: &[u8],
    capture: &[u8],
    profile: &[u8],
    candidate: &CalibrationCandidate,
) -> DeviceProfile {
    let confirmation = PromotionConfirmation {
        source_capture_digest: candidate.source_capture_digest.clone(),
        candidate_digest: canonical_candidate_digest(candidate).expect("candidate digest"),
    };
    promote_calibration_candidate(contract, capture, profile, candidate, &confirmation)
        .expect("exact promotion")
}

fn assert_candidate_physical_trace_matches(
    apple: &CalibrationCandidate,
    browser: &CalibrationCandidate,
) {
    let physical = capture();
    for apple_axis in &apple.axes {
        let web_axis = browser
            .axes
            .iter()
            .find(|axis| axis.source_index == apple_axis.source_index)
            .expect("browser source axis");
        assert_eq!(apple_axis.invert, web_axis.invert);
        assert_eq!(apple_axis.proposed_deadzone, web_axis.proposed_deadzone);
        let apple_config = candidate_axis_config(apple_axis);
        let web_config = candidate_axis_config(web_axis);
        for sample in &physical.samples {
            let raw = sample.axes[apple_axis.source_index];
            let divisor = if apple_axis.source_index == 0 {
                1000.0
            } else {
                500.0
            };
            let browser_raw = raw / divisor - 1.0;
            let native = normalize_axis(raw, &apple_config).value;
            let web = normalize_axis(browser_raw, &web_config).value;
            assert!(
                (native - web).abs() < 1.0e-6,
                "source {} physical sample differs",
                apple_axis.source_index
            );
        }
    }
}

fn candidate_axis_config(axis: &AxisCharacterization) -> AxisConfig {
    AxisConfig {
        source_index: axis.source_index,
        logical: axis.logical.clone(),
        invert: axis.invert,
        deadzone: axis.proposed_deadzone,
        expo: 0.0,
        calibration: axis.calibration,
    }
}

fn profile_from_web_golden() -> DeviceProfile {
    let group = web_golden_group();
    serde_json::from_value(group["steps"][0]["addDeviceProfile"]["profile"].clone())
        .expect("promoted web profile")
}

fn assert_web_golden_uses_complete_browser_trace() {
    let capture: crate::CharacterizationCapture =
        serde_json::from_slice(BROWSER_CAPTURE_BYTES).expect("browser capture");
    let group = web_golden_group();
    let steps = group["steps"].as_array().expect("golden steps");
    let trace = steps
        .get(steps.len().saturating_sub(capture.samples.len())..)
        .expect("complete golden trace");
    assert_eq!(trace.len(), capture.samples.len());
    for (step, sample) in trace.iter().zip(capture.samples) {
        let axes: Vec<f32> =
            serde_json::from_value(step["pad"]["axes"].clone()).expect("golden axes");
        assert_eq!(axes, sample.axes);
    }
}

fn web_golden_group() -> serde_json::Value {
    let document: serde_json::Value = serde_json::from_slice(WEB_GOLDEN_BYTES).expect("web golden");
    document["groups"]
        .as_array()
        .expect("golden groups")
        .iter()
        .find(|group| {
            group["name"]
                == "accepted HID candidates normalize identically in native and wasm paths"
        })
        .expect("HID golden group")
        .clone()
}

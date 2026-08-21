use super::{
    CAPTURE_BYTES, CONTRACT_BYTES, PROFILE_BYTES, candidate, capture, characterize, digest_hex,
};
use crate::{
    AnalysisError, CharacterizationError, DeadzoneEvidenceMethod, DeadzoneEvidenceStatus,
    PromotionConfirmation, RawReportDecoder, SamplingSource, SourceAxisContract, TimingObservation,
    canonical_candidate_digest, characterize_capture, promote_calibration_candidate,
};

#[test]
fn synthetic_fixture_cannot_be_promoted() {
    let candidate = candidate();
    let confirmation = PromotionConfirmation {
        source_capture_digest: candidate.source_capture_digest.clone(),
        candidate_digest: canonical_candidate_digest(&candidate).expect("candidate digest"),
    };
    assert!(matches!(
        promote_calibration_candidate(
            CONTRACT_BYTES,
            CAPTURE_BYTES,
            PROFILE_BYTES,
            &candidate,
            &confirmation,
        ),
        Err(CharacterizationError::UnsupportedPromotionSource {
            sampling_source: SamplingSource::Synthetic
        })
    ));
}

#[test]
fn native_provenance_requires_a_contract_decoder() {
    let mut capture = capture();
    capture.source = SamplingSource::NativeHid;
    capture.timing_observation = TimingObservation::ReportCallbacks;
    capture.deadzone_evidence.status = DeadzoneEvidenceStatus::NotObserved;
    capture.deadzone_evidence.method = DeadzoneEvidenceMethod::RawHidReports;
    capture.deadzone_evidence.sample_count =
        u64::try_from(capture.samples.len()).expect("sample count");
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::InvalidCapture { .. })
    ));
    capture.source = SamplingSource::AppleHid;
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::InvalidCapture { .. })
    ));
}

#[test]
fn native_raw_reports_are_bound_to_every_recorded_axis() {
    let (contract_bytes, capture_bytes) = native_evidence();
    let candidate = characterize_capture(CONTRACT_BYTES, &capture_bytes, PROFILE_BYTES)
        .expect_err("the wrong contract must fail");
    assert!(matches!(
        candidate,
        AnalysisError::ContractDigestMismatch { .. }
    ));
    let candidate = characterize_capture(&contract_bytes, &capture_bytes, PROFILE_BYTES)
        .expect("bound native candidate");
    let confirmation = PromotionConfirmation {
        source_capture_digest: candidate.source_capture_digest.clone(),
        candidate_digest: canonical_candidate_digest(&candidate).expect("candidate digest"),
    };
    assert!(
        promote_calibration_candidate(
            &contract_bytes,
            &capture_bytes,
            PROFILE_BYTES,
            &candidate,
            &confirmation,
        )
        .is_ok()
    );

    let mut changed_axes: crate::CharacterizationCapture =
        serde_json::from_slice(&capture_bytes).expect("native capture");
    changed_axes.samples[0].axes[0] += 1.0;
    assert!(matches!(
        characterize_capture(
            &contract_bytes,
            &serde_json::to_vec(&changed_axes).expect("changed capture"),
            PROFILE_BYTES,
        ),
        Err(AnalysisError::InvalidCapture { .. })
    ));

    let mut changed_report: crate::CharacterizationCapture =
        serde_json::from_slice(&capture_bytes).expect("native capture");
    changed_report.samples[0].report_hex = Some("01 00 f4 01".to_owned());
    assert!(matches!(
        characterize_capture(
            &contract_bytes,
            &serde_json::to_vec(&changed_report).expect("changed capture"),
            PROFILE_BYTES,
        ),
        Err(AnalysisError::InvalidCapture { .. })
    ));

    changed_report.samples[0].report_hex = Some("00 ff".to_owned());
    assert!(matches!(
        characterize_capture(
            &contract_bytes,
            &serde_json::to_vec(&changed_report).expect("short report capture"),
            PROFILE_BYTES,
        ),
        Err(AnalysisError::InvalidCapture { .. })
    ));

    changed_report.samples[0].report_hex = Some("0G".to_owned());
    assert!(matches!(
        characterize_capture(
            &contract_bytes,
            &serde_json::to_vec(&changed_report).expect("invalid report capture"),
            PROFILE_BYTES,
        ),
        Err(AnalysisError::InvalidCapture { .. })
    ));
}

#[test]
fn report_layout_rejects_overlap_and_a_wrong_report_id() {
    let mut contract: SourceAxisContract =
        serde_json::from_slice(CONTRACT_BYTES).expect("source contract");
    contract.raw_report_layout = Some(crate::RawReportLayout {
        report_byte_count: 5,
        report_id: Some(7),
        axes: vec![
            crate::RawReportAxisField {
                source_index: 0,
                bit_offset: 8,
                bit_width: 16,
                signed: false,
            },
            crate::RawReportAxisField {
                source_index: 1,
                bit_offset: 16,
                bit_width: 16,
                signed: false,
            },
        ],
    });
    assert!(matches!(
        RawReportDecoder::new(&contract),
        Err(crate::RawReportError::OverlappingAxisField { .. })
    ));
    contract.raw_report_layout.as_mut().expect("layout").axes[1].bit_offset = 24;
    let decoder = RawReportDecoder::new(&contract).expect("valid layout");
    assert!(matches!(
        decoder.decode(&[6, 0, 0, 0, 0]),
        Err(crate::RawReportError::ReportIdMismatch { .. })
    ));
}

#[test]
fn capture_actions_reject_unknown_nested_fields() {
    for segment in [0, 1] {
        let mut value: serde_json::Value =
            serde_json::from_slice(CAPTURE_BYTES).expect("capture value");
        value["segments"][segment]["action"]["vehicle_limit"] = serde_json::json!(10);
        let bytes = serde_json::to_vec(&value).expect("capture bytes");
        assert!(matches!(
            characterize_capture(CONTRACT_BYTES, &bytes, PROFILE_BYTES),
            Err(AnalysisError::CaptureParse { .. })
        ));
    }
}

#[test]
fn characterization_refuses_the_registry_wildcard_identity() {
    let mut contract: serde_json::Value =
        serde_json::from_slice(CONTRACT_BYTES).expect("contract value");
    contract["device"]["vendor_id"] = serde_json::json!(0);
    contract["device"]["product_id"] = serde_json::json!(0);
    let contract_bytes = serde_json::to_vec(&contract).expect("contract bytes");

    let mut capture = capture();
    capture.device.vendor_id = 0;
    capture.device.product_id = 0;
    capture.source_contract_digest = digest_hex(&contract_bytes);
    let capture_bytes = serde_json::to_vec(&capture).expect("capture bytes");
    let mut profile: serde_json::Value =
        serde_json::from_slice(PROFILE_BYTES).expect("profile value");
    profile["device"]["vendor_id"] = serde_json::json!(0);
    profile["device"]["product_id"] = serde_json::json!(0);
    let profile_bytes = serde_json::to_vec(&profile).expect("profile bytes");

    assert!(matches!(
        characterize_capture(&contract_bytes, &capture_bytes, &profile_bytes),
        Err(AnalysisError::ContractMismatch { .. })
    ));
}

#[test]
fn source_axis_contract_comparison_preserves_signed_zero() {
    let mut capture = capture();
    assert_eq!(capture.source_axes[0].minimum.to_bits(), 0.0f32.to_bits());
    capture.source_axes[0].minimum = -0.0;
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::ContractMismatch { .. })
    ));
}

#[test]
fn characterization_rejects_an_oversized_product_name() {
    let mut capture = capture();
    capture.device.product = Some("é".repeat(129));
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::ContractMismatch { .. })
    ));
}

#[test]
fn non_native_sources_cannot_attach_raw_reports() {
    let mut capture = capture();
    capture.samples[0].report_hex = Some("00".to_owned());
    assert!(matches!(
        characterize(&capture),
        Err(AnalysisError::InvalidCapture { .. })
    ));
}

fn native_evidence() -> (Vec<u8>, Vec<u8>) {
    let mut contract: SourceAxisContract =
        serde_json::from_slice(CONTRACT_BYTES).expect("source contract");
    contract.raw_report_layout = Some(crate::RawReportLayout {
        report_byte_count: 4,
        report_id: None,
        axes: vec![
            crate::RawReportAxisField {
                source_index: 0,
                bit_offset: 0,
                bit_width: 16,
                signed: false,
            },
            crate::RawReportAxisField {
                source_index: 1,
                bit_offset: 16,
                bit_width: 16,
                signed: false,
            },
        ],
    });
    let contract_bytes = serde_json::to_vec(&contract).expect("native contract bytes");
    let mut capture = capture();
    capture.source = SamplingSource::NativeHid;
    capture.timing_observation = TimingObservation::ReportCallbacks;
    capture.deadzone_evidence.status = DeadzoneEvidenceStatus::NotObserved;
    capture.deadzone_evidence.method = DeadzoneEvidenceMethod::RawHidReports;
    capture.deadzone_evidence.sample_count =
        u64::try_from(capture.samples.len()).expect("sample count");
    capture.source_contract_digest = digest_hex(&contract_bytes);
    for sample in &mut capture.samples {
        let first = (sample.axes[0] as u16).to_le_bytes();
        let second = (sample.axes[1] as u16).to_le_bytes();
        sample.report_hex = Some(format!(
            "{:02x} {:02x} {:02x} {:02x}",
            first[0], first[1], second[0], second[1]
        ));
    }
    (
        contract_bytes,
        serde_json::to_vec(&capture).expect("native capture bytes"),
    )
}

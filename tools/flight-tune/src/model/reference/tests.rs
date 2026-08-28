#![allow(clippy::expect_used)]

use pilotage_mission_core::{Digest as MissionDigest, MissionDocument};

use super::MissionReference;
use crate::{TuneError, calibration_mission_document, reference_observation_scenario};

const RECEIPT_TIMEOUT_NS: u64 = 20_000_000;
const MAX_SAMPLES: u32 = 16;

#[test]
fn a_reference_repeats_the_document_identity_and_receipt_timeout() {
    let document = mission("unit-mission", 0);
    let reference = reference(&document);

    assert_eq!(reference.revision_id, document.identity.revision_id);
    assert_eq!(reference.schema_version, document.identity.schema_version);
    assert_eq!(
        *reference.content_digest.as_bytes(),
        *document.identity.content_digest.as_bytes()
    );
    assert_eq!(
        reference.sample_timeout_ns,
        document.execution_policy.receipt_timeout_ns
    );
    assert!(reference.verify_document(&document).is_ok());
}

#[test]
fn a_changed_mission_document_changes_the_reference_identity() {
    let first = reference(&mission("unit-mission", 0));
    let second = reference(&mission("unit-mission", 1));

    assert_eq!(first.revision_id, second.revision_id);
    assert_ne!(first.content_digest, second.content_digest);
}

#[test]
fn a_reference_refuses_a_document_with_another_revision() {
    let reference = reference(&mission("unit-mission", 0));

    assert_mismatch(reference.verify_document(&mission("other-mission", 0)));
}

#[test]
fn a_reference_refuses_a_document_with_another_schema_version() {
    let document = mission("unit-mission", 0);
    let reference = reference(&document);
    let mut changed = document;
    changed.identity.schema_version = changed.identity.schema_version.wrapping_add(1);

    assert_mismatch(reference.verify_document(&changed));
}

#[test]
fn a_reference_refuses_a_document_with_other_content() {
    let reference = reference(&mission("unit-mission", 0));

    assert_mismatch(reference.verify_document(&mission("unit-mission", 1)));
}

#[test]
fn a_reference_recalculates_the_declared_content_digest() {
    let mut tampered = mission("unit-mission", 0);
    tampered.identity.content_digest = MissionDigest::from_bytes([9; 32]);
    let reference = reference(&tampered);

    assert_eq!(
        *reference.content_digest.as_bytes(),
        *tampered.identity.content_digest.as_bytes()
    );
    assert_mismatch(reference.verify_document(&tampered));
}

#[test]
fn a_reference_refuses_a_sample_timeout_that_the_document_does_not_carry() {
    let document = mission("unit-mission", 0);
    let mut reference = reference(&document);
    reference.sample_timeout_ns = reference.sample_timeout_ns.wrapping_add(1_000_000);

    assert_mismatch(reference.verify_document(&document));
}

#[test]
fn a_reference_needs_run_limits_inside_their_range() {
    let document = mission("unit-mission", 0);

    assert!(MissionReference::from_document(&document, 0).is_err());
    let mut reference = reference(&document);
    reference.sample_timeout_ns = 0;
    assert!(reference.validate().is_err());
    reference.sample_timeout_ns = 60_000_000_001;
    assert!(reference.validate().is_err());
}

#[test]
fn run_duration_covers_every_permitted_sample() {
    let reference = reference(&mission("unit-mission", 0));

    assert_eq!(
        reference.run_duration_ns(),
        RECEIPT_TIMEOUT_NS * u64::from(MAX_SAMPLES)
    );
}

fn mission(id: &str, retry_limit: u16) -> MissionDocument {
    calibration_mission_document(
        &reference_observation_scenario(id, None),
        retry_limit,
        RECEIPT_TIMEOUT_NS,
    )
    .expect("mission document")
}

fn reference(document: &MissionDocument) -> MissionReference {
    MissionReference::from_document(document, MAX_SAMPLES).expect("mission reference")
}

fn assert_mismatch(result: Result<(), TuneError>) {
    assert!(matches!(result, Err(TuneError::ReceiptMismatch { .. })));
}

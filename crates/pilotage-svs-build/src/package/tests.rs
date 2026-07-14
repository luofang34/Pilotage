//! Tests for package assembly: identity, accuracy, and use-restriction union.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_svs_db::{UseRestrictions, manifest_content_hash};

use crate::build_package;
use crate::fixtures;
use crate::source::LicenseCode;

#[test]
fn active_id_is_content_addressed() {
    let artifact = build_package(&fixtures::config(), &fixtures::dataset()).expect("build");
    let manifest = &artifact.package.manifest;
    assert_eq!(
        manifest.active_id().content_hash,
        manifest_content_hash(manifest)
    );
}

#[test]
fn worst_case_accuracy_is_recorded() {
    let mut dataset = fixtures::dataset();
    dataset.meta[1].accuracy.horizontal_mm = 9_000;
    let artifact = build_package(&fixtures::config(), &dataset).expect("build");
    assert_eq!(
        artifact.package.manifest.content.accuracy.horizontal_mm,
        9_000
    );
}

#[test]
fn restrictions_union_over_sources() {
    let mut dataset = fixtures::dataset();
    dataset.meta[2].license = LicenseCode::TrainingOnly;
    let artifact = build_package(&fixtures::config(), &dataset).expect("build");
    let restrictions = artifact.package.manifest.provenance.restrictions;
    assert!(restrictions.contains(UseRestrictions::TRAINING_ONLY));
    assert!(
        restrictions.contains(UseRestrictions::NO_OPERATIONAL_USE),
        "the base SIM restriction is always present"
    );
}

#[test]
fn processing_chain_records_the_tool() {
    let artifact = build_package(&fixtures::config(), &fixtures::dataset()).expect("build");
    let steps = artifact.package.manifest.provenance.processing.steps();
    assert_eq!(steps.len(), crate::chain::STAGE_CODES.len());
    assert!(
        steps
            .iter()
            .all(|s| s.tool_id == crate::provenance::TOOL_ID)
    );
}

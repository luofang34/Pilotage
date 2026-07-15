//! Tests for independent artifact verification: bundle tamper-evidence and the
//! decode-based report cross-check.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod dispositions;
mod source_binding;

use ed25519_dalek::{Signer, SigningKey};
use pilotage_svs_db::{DayNumber, TrustAnchor, TrustRoot, UsePolicy};

use super::{
    decode_package_reports, verify_artifact, verify_artifact_with_sources, verify_source_digests,
};
use crate::bundle::canonical_bundle_bytes;
use crate::chain::{BuildArtifact, build_package};
use crate::error::{BuildError, VerifyError};
use crate::fixtures;
use crate::provenance::{RecordKey, RecordLineage, SourceSummary};
use crate::source::{LicenseCode, SourceId, SourceRecordRef};

/// A trust root that trusts the fixture signing key.
fn trust_root() -> TrustRoot {
    let config = fixtures::config();
    let key = SigningKey::from_bytes(&config.signing.signing_seed);
    TrustRoot::new(vec![TrustAnchor {
        key_id: config.signing.key_id,
        public_key: key.verifying_key().to_bytes(),
    }])
}

/// Re-signs the bundle so a mutation isolates the check under test rather than
/// tripping the signature first.
fn resign_bundle(artifact: &mut BuildArtifact) {
    let bytes = canonical_bundle_bytes(&artifact.provenance, &artifact.reports).unwrap();
    let key = SigningKey::from_bytes(&fixtures::config().signing.signing_seed);
    artifact.bundle_signature = key.sign(&bytes).to_bytes();
}

fn built() -> BuildArtifact {
    build_package(&fixtures::config(), &fixtures::dataset()).expect("build")
}

#[test]
fn verify_artifact_accepts_a_clean_build() {
    let artifact = built();
    verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    )
    .expect("a clean artifact verifies");
}

#[test]
fn mutated_provenance_fails_bundle_signature() {
    let mut artifact = built();
    // Flip one byte of the provenance (a stage count); do NOT re-sign.
    artifact.provenance.stages[0].outputs ^= 1;
    let result = verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(
        matches!(result, Err(VerifyError::BundleSignatureInvalid)),
        "altering provenance must break the bundle signature: {result:?}"
    );
}

#[test]
fn mutated_report_fails_bundle_signature() {
    let mut artifact = built();
    artifact.reports.quality.outliers_rejected ^= 1;
    let result = verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(matches!(result, Err(VerifyError::BundleSignatureInvalid)));
}

#[test]
fn broken_binding_is_detected() {
    let mut artifact = built();
    // Point the provenance at a different package, then re-sign so the signature
    // passes and the binding check is what fails.
    artifact.provenance.package_content_hash = [0u8; 32];
    resign_bundle(&mut artifact);
    let result = verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(matches!(result, Err(VerifyError::BundleBindingMismatch)));
}

#[test]
fn report_disagreeing_with_package_is_detected() {
    let mut artifact = built();
    // Claim more terrain posts than the package holds, re-sign so the signature
    // and binding pass, leaving only the decode cross-check to catch it.
    artifact.reports.coverage.terrain_posts += 1;
    resign_bundle(&mut artifact);
    let result = verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(
        matches!(
            result,
            Err(VerifyError::ReportMismatch {
                field: "terrain_posts"
            })
        ),
        "a report disagreeing with the decoded package must be caught: {result:?}"
    );
}

#[test]
fn decoded_reports_match_the_pipeline() {
    let artifact = built();
    let decoded = decode_package_reports(&artifact).expect("decode");
    let coverage = &artifact.reports.coverage;
    assert_eq!(decoded.terrain_tiles, coverage.terrain_tiles);
    assert_eq!(decoded.obstacle_tiles, coverage.obstacle_tiles);
    assert_eq!(decoded.aerodrome_tiles, coverage.aerodrome_tiles);
    assert_eq!(decoded.runway_tiles, coverage.runway_tiles);
    assert_eq!(decoded.terrain_posts, coverage.terrain_posts);
    assert_eq!(decoded.obstacles, coverage.obstacles);
    assert_eq!(decoded.covered_nodes, coverage.covered_nodes);
    assert!(decoded.seam_ok);
}

#[test]
fn untrusted_key_fails_bundle_verification() {
    let artifact = built();
    let wrong = TrustRoot::new(vec![TrustAnchor {
        key_id: fixtures::config().signing.key_id,
        public_key: SigningKey::from_bytes(&[9u8; 32])
            .verifying_key()
            .to_bytes(),
    }]);
    let result = verify_artifact(
        &artifact,
        &wrong,
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    // The package signature is checked first and already fails against the wrong
    // key, so verification is refused before the bundle is reached.
    assert!(result.is_err());
}

#[test]
fn record_lineage_is_a_total_bijection() {
    let artifact = built();
    verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    )
    .expect("the clean build's records and lineage are in 1:1 correspondence");
    // Every emitted record has exactly one lineage entry: the lineage count
    // equals terrain posts + obstacles + aerodromes + runways.
    let decoded = decode_package_reports(&artifact).unwrap();
    let expected = decoded.terrain_posts + decoded.obstacles + 1 /* aerodrome */ + 1 /* runway */;
    assert_eq!(artifact.provenance.records.len() as u32, expected);
}

#[test]
fn package_record_without_a_lineage_entry_fails() {
    let mut artifact = built();
    // Drop one lineage entry, then re-sign so the bundle passes and only the
    // bijection can catch the now-untraceable package record.
    artifact.provenance.records.remove(0);
    resign_bundle(&mut artifact);
    let result = verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(
        matches!(result, Err(VerifyError::LineageMissingForRecord { .. })),
        "a package record with no lineage entry must fail: {result:?}"
    );
}

#[test]
fn lineage_entry_without_a_package_record_fails() {
    let mut artifact = built();
    // Add a lineage entry for a record the package does not contain.
    artifact.provenance.records.push(RecordLineage {
        class: pilotage_svs_db::FeatureClass::Terrain.to_u8(),
        lat_index: 999,
        lon_index: 999,
        key: RecordKey::TerrainNode { i: 999, j: 999 },
        sources: vec![],
    });
    resign_bundle(&mut artifact);
    let result = verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(
        matches!(result, Err(VerifyError::LineageOrphan { .. })),
        "a lineage entry with no package record must fail: {result:?}"
    );
}

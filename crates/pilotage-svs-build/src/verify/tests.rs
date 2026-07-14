//! Tests for independent artifact verification: bundle tamper-evidence and the
//! decode-based report cross-check.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use ed25519_dalek::{Signer, SigningKey};
use pilotage_svs_db::{DayNumber, TrustAnchor, TrustRoot, UsePolicy};

use super::{decode_package_reports, verify_artifact, verify_source_digests};
use crate::bundle::canonical_bundle_bytes;
use crate::chain::{BuildArtifact, build_package};
use crate::error::VerifyError;
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

#[test]
fn source_digest_changes_when_input_changes() {
    let a = built();
    let mut dataset = fixtures::dataset();
    dataset.terrain[0].posts[5] = Some(4321.0);
    let b = build_package(&fixtures::config(), &dataset).expect("build b");
    let digest_a = a.provenance.sources[0].content_digest;
    let digest_b = b.provenance.sources[0].content_digest;
    assert_ne!(
        digest_a, digest_b,
        "changing one source input byte must change its recorded digest"
    );
    assert_ne!(
        a.bundle_signature, b.bundle_signature,
        "the changed digest must change the signed bundle"
    );
}

#[test]
fn source_digest_matches_its_own_input() {
    let artifact = built();
    verify_source_digests(&artifact, &fixtures::dataset())
        .expect("recorded digests match the source input");
}

#[test]
fn mismatched_source_digest_is_rejected() {
    let artifact = built();
    // A source input differing by one byte no longer matches the recorded digest.
    let mut altered = fixtures::dataset();
    altered.terrain[0].posts[5] = Some(4321.0);
    let result = verify_source_digests(&artifact, &altered);
    assert!(
        matches!(result, Err(VerifyError::SourceDigestMismatch { .. })),
        "a provenance whose source digest does not match the input must be rejected: {result:?}"
    );
}

/// Verifies a mutated, re-signed artifact, so only the source-reference checks
/// can be what fails.
fn verify_resigned(artifact: &mut BuildArtifact) -> Result<(), VerifyError> {
    resign_bundle(artifact);
    verify_artifact(
        artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    )
}

#[test]
fn empty_lineage_sources_fail() {
    let mut artifact = built();
    artifact.provenance.records[0].sources.clear();
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(result, Err(VerifyError::EmptyLineageSources { .. })),
        "a lineage record with no source must fail: {result:?}"
    );
}

#[test]
fn unknown_lineage_source_fails() {
    let mut artifact = built();
    artifact.provenance.records[0].sources = vec![SourceRecordRef {
        source: SourceId(999),
        record: 0,
    }];
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(
            result,
            Err(VerifyError::UnknownLineageSource { source_id: 999 })
        ),
        "a dangling source reference must fail: {result:?}"
    );
}

#[test]
fn duplicated_lineage_source_fails() {
    let mut artifact = built();
    let dup = SourceRecordRef {
        source: fixtures::TERRAIN_SRC,
        record: 0,
    };
    artifact.provenance.records[0].sources = vec![dup, dup];
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(result, Err(VerifyError::DuplicateLineageSource { .. })),
        "a duplicated source reference must fail: {result:?}"
    );
}

#[test]
fn out_of_range_lineage_source_fails() {
    let mut artifact = built();
    artifact.provenance.records[0].sources = vec![SourceRecordRef {
        source: fixtures::TERRAIN_SRC,
        record: 999_999,
    }];
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(result, Err(VerifyError::SourceRecordOutOfRange { .. })),
        "an out-of-range source record index must fail: {result:?}"
    );
}

#[test]
fn summary_referenced_by_no_lineage_fails() {
    let mut artifact = built();
    artifact.provenance.sources.push(SourceSummary {
        id: SourceId(999),
        version: 1,
        content_digest: [0u8; 32],
        license: LicenseCode::Open,
        horizontal_datum: 1,
        vertical_datum: 1,
        accuracy_h_mm: 0,
        accuracy_v_mm: 0,
        record_count: 1,
    });
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(
            result,
            Err(VerifyError::UnreferencedSourceSummary { source_id: 999 })
        ),
        "a summary referenced by no lineage must fail: {result:?}"
    );
}

#[test]
fn dataset_source_without_summary_fails() {
    let artifact = built();
    // A dataset with an extra source the provenance summaries do not cover.
    let mut extra = fixtures::dataset();
    extra
        .meta
        .push(fixtures::meta(SourceId(9), LicenseCode::Open));
    let result = verify_source_digests(&artifact, &extra);
    assert!(
        matches!(
            result,
            Err(VerifyError::SourceSummaryMissing { source_id: 9 })
        ),
        "a dataset source with no summary must fail: {result:?}"
    );
}

#[test]
fn source_set_bijection_holds_on_a_clean_build() {
    let artifact = built();
    verify_artifact(
        &artifact,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    )
    .expect("every lineage source is valid and every summary is referenced");
    verify_source_digests(&artifact, &fixtures::dataset())
        .expect("digests cover every source, both sides");
    assert_eq!(
        artifact.provenance.sources.len(),
        fixtures::dataset().meta.len(),
        "the summary set equals the dataset source set"
    );
}

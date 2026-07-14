//! Tests for independent artifact verification: bundle tamper-evidence and the
//! decode-based report cross-check.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use ed25519_dalek::{Signer, SigningKey};
use pilotage_svs_db::{DayNumber, TrustAnchor, TrustRoot, UsePolicy};

use super::{decode_package_reports, verify_artifact};
use crate::bundle::canonical_bundle_bytes;
use crate::chain::{BuildArtifact, build_package};
use crate::error::VerifyError;
use crate::fixtures;

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

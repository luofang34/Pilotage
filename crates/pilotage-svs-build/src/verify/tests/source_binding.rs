//! Tests for the source bindings: content digests, summary/meta duplication,
//! record-reference resolution, and the combined verification entry point.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::*;

use crate::provenance::{Disposition, RecordDisposition};

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
    artifact.provenance.records[0].sources = vec![SourceRecordRef::obstacle(SourceId(999), 0)];
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
    let dup = SourceRecordRef::terrain(fixtures::TERRAIN_SRC, 39.5, -75.5, 1, 1);
    artifact.provenance.records[0].sources = vec![dup, dup];
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(result, Err(VerifyError::DuplicateLineageSource { .. })),
        "a duplicated source reference must fail: {result:?}"
    );
}

#[test]
fn duplicate_source_summary_fails() {
    let mut artifact = built();
    // List the first source twice; source identity is now ambiguous.
    let duplicate = artifact.provenance.sources[0];
    artifact.provenance.sources.push(duplicate);
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(result, Err(VerifyError::DuplicateSourceSummary { .. })),
        "a source listed twice in the signed provenance must fail: {result:?}"
    );
}

#[test]
fn unresolved_lineage_source_ref_is_rejected_against_dataset() {
    let mut artifact = built();
    // A terrain node outside the source grid resolves to no dataset record. The
    // source still has a summary, so only resolution against the dataset catches
    // it.
    artifact.provenance.records[0].sources = vec![SourceRecordRef::terrain(
        fixtures::TERRAIN_SRC,
        39.5,
        -75.5,
        999,
        999,
    )];
    resign_bundle(&mut artifact);
    let result = verify_source_digests(&artifact, &fixtures::dataset());
    assert!(
        matches!(result, Err(VerifyError::UnresolvedSourceRef { .. })),
        "a lineage ref resolving to no source record must fail: {result:?}"
    );
}

#[test]
fn ambiguous_source_records_are_rejected() {
    // A dataset where two distinct obstacles share one reference: the reference
    // cannot name exactly one source record.
    let mut ambiguous = fixtures::dataset();
    let shared = SourceRecordRef::obstacle(fixtures::OBSTACLE_SRC, 0);
    ambiguous.obstacles = vec![
        crate::source::Obstacle {
            lat_deg: 40.2,
            lon_deg: -74.7,
            height_m: 50.0,
            kind: crate::source::ObstacleKind::Tower,
            source: shared,
        },
        crate::source::Obstacle {
            lat_deg: 40.25,
            lon_deg: -74.65,
            height_m: 60.0,
            kind: crate::source::ObstacleKind::Mast,
            source: shared,
        },
    ];
    // The build self-verifies end to end and withholds the artifact: the
    // ambiguity is caught at build time, so nothing is emitted for a
    // downstream verifier to have to catch.
    let result = build_package(&fixtures::config(), &ambiguous);
    assert!(
        matches!(
            result,
            Err(BuildError::ArtifactSelfVerification {
                source: VerifyError::AmbiguousSourceRecord { .. }
            })
        ),
        "two distinct source records sharing a reference must fail the build closed: {result:?}"
    );
}

#[test]
fn a_duplicated_source_meta_fails_the_build_closed() {
    // Two metadata entries for one source id make the governing datum,
    // license, and version ambiguous; the build refuses rather than letting
    // meta_for silently pick whichever came first.
    let mut dataset = fixtures::dataset();
    let mut twin = dataset.meta[0];
    twin.version = twin.version.wrapping_add(1);
    let duplicated = dataset.meta[0].id;
    dataset.meta.push(twin);
    let result = build_package(&fixtures::config(), &dataset);
    assert!(
        matches!(
            result,
            Err(BuildError::DuplicateSourceIdentity { source_id }) if source_id == duplicated.0
        ),
        "a duplicated source id must fail the build closed: {result:?}"
    );
}

#[test]
fn a_duplicated_source_meta_fails_source_verification() {
    // The artifact is clean, but the dataset presented at verification time
    // duplicates a source id: the digest check would silently bind to the
    // first entry, so the duplication itself is refused.
    let artifact = built();
    let mut dataset = fixtures::dataset();
    let twin = dataset.meta[0];
    let duplicated = twin.id;
    dataset.meta.push(twin);
    let result = verify_source_digests(&artifact, &dataset);
    assert!(
        matches!(
            result,
            Err(VerifyError::DuplicateSourceMeta { source_id }) if source_id == duplicated.0
        ),
        "a duplicated dataset source id must fail verification: {result:?}"
    );
}

#[test]
fn the_combined_entry_point_verifies_sources_too() {
    // verify_artifact_with_sources accepts a clean artifact with its true
    // dataset, and rejects the same artifact against a tampered dataset even
    // though the artifact alone verifies — source verification cannot be
    // skipped by forgetting a second call.
    let artifact = built();
    let config = fixtures::config();
    let key = SigningKey::from_bytes(&config.signing.signing_seed);
    let trust = TrustRoot::new(vec![TrustAnchor {
        key_id: config.signing.key_id,
        public_key: key.verifying_key().to_bytes(),
    }]);
    let clean = verify_artifact_with_sources(
        &artifact,
        &fixtures::dataset(),
        &trust,
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(clean.is_ok(), "clean artifact + true dataset: {clean:?}");

    let mut tampered = fixtures::dataset();
    tampered.obstacles[0].height_m += 1.0;
    let result = verify_artifact_with_sources(
        &artifact,
        &tampered,
        &trust,
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(
        matches!(result, Err(VerifyError::SourceDigestMismatch { .. })),
        "a tampered dataset must fail the combined verification: {result:?}"
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

#[test]
fn a_runway_with_an_unknown_datum_source_fails_the_build() {
    // The runway carries its own source record: its datum is validated in its
    // own right, never inherited from the enclosing aerodrome's source.
    let mut dataset = fixtures::dataset();
    let mut rogue = fixtures::meta(SourceId(9), LicenseCode::Open);
    rogue.horizontal_datum = pilotage_geo::HorizontalDatum::Unknown;
    dataset.meta.push(rogue);
    dataset.aerodromes[0].runways[0].source =
        SourceRecordRef::runway(SourceId(9), fixtures::AERODROME_IDENT, 0x0918);
    let result = build_package(&fixtures::config(), &dataset);
    assert!(
        matches!(
            result,
            Err(BuildError::UnknownSourceDatum { source_id: 9, .. })
        ),
        "a runway from an Unknown-datum source must fail the build: {result:?}"
    );
}

#[test]
fn runway_bytes_bind_to_their_own_source_digest() {
    // A runway owned by source B under an aerodrome of source A belongs to B's
    // digest and record count, never A's.
    let mut dataset = fixtures::dataset();
    dataset.aerodromes[0].runways[0].source =
        SourceRecordRef::runway(fixtures::OBSTACLE_SRC, fixtures::AERODROME_IDENT, 0x0918);
    let aero_meta = *dataset.meta_for(fixtures::AERODROME_SRC).expect("meta");
    let rwy_meta = *dataset.meta_for(fixtures::OBSTACLE_SRC).expect("meta");
    let aero_before = crate::source::source_content_digest(&dataset, &aero_meta);
    let rwy_before = crate::source::source_content_digest(&dataset, &rwy_meta);
    dataset.aerodromes[0].runways[0].end_a_lat_deg += 0.001;
    let aero_after = crate::source::source_content_digest(&dataset, &aero_meta);
    let rwy_after = crate::source::source_content_digest(&dataset, &rwy_meta);
    assert_eq!(
        aero_before, aero_after,
        "the aerodrome source's digest must not absorb another source's runway"
    );
    assert_ne!(
        rwy_before, rwy_after,
        "the runway's own source digest must cover the runway bytes"
    );
}

#[test]
fn a_disposition_for_a_nonexistent_record_fails_the_full_verifier() {
    // A disposition claiming a fate for a record the dataset never contained
    // is fabricated provenance; the combined verifier refuses it even after a
    // clean re-sign.
    let mut artifact = built();
    artifact.provenance.dispositions.push(RecordDisposition {
        source: SourceRecordRef::obstacle(fixtures::OBSTACLE_SRC, 999),
        disposition: Disposition::Clipped,
    });
    resign_bundle(&mut artifact);
    let config = fixtures::config();
    let key = SigningKey::from_bytes(&config.signing.signing_seed);
    let trust = TrustRoot::new(vec![TrustAnchor {
        key_id: config.signing.key_id,
        public_key: key.verifying_key().to_bytes(),
    }]);
    let result = verify_artifact_with_sources(
        &artifact,
        &fixtures::dataset(),
        &trust,
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(
        matches!(result, Err(VerifyError::DispositionInvalid { .. })),
        "a disposition naming no dataset record must fail: {result:?}"
    );
}

#[test]
fn a_duplicate_or_contradictory_disposition_fails_artifact_verification() {
    // Two dispositions for one record are ambiguous provenance.
    let mut artifact = built();
    let target = SourceRecordRef::obstacle(fixtures::OBSTACLE_SRC, 0);
    artifact.provenance.dispositions.push(RecordDisposition {
        source: target,
        disposition: Disposition::Clipped,
    });
    artifact.provenance.dispositions.push(RecordDisposition {
        source: target,
        disposition: Disposition::Clipped,
    });
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(result, Err(VerifyError::DispositionInvalid { .. })),
        "duplicate dispositions must fail: {result:?}"
    );

    // A no-contribution claim for a record the output lineage draws from is a
    // contradiction.
    let mut artifact = built();
    let drawn = artifact.provenance.records[0].sources[0];
    artifact.provenance.dispositions.push(RecordDisposition {
        source: drawn,
        disposition: Disposition::NoContribution {
            reason: "fabricated",
        },
    });
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(result, Err(VerifyError::DispositionInvalid { .. })),
        "a contradictory no-contribution disposition must fail: {result:?}"
    );
}

#[test]
fn the_aerodrome_stage_ledger_counts_runways_as_inputs() {
    // The pipeline consumes the aerodrome AND its runway, and emits both: the
    // stage ledger must conserve (fixture: 1 aerodrome + 1 runway = 2 inputs).
    let artifact = built();
    let stage = artifact
        .provenance
        .stages
        .iter()
        .find(|s| s.name == "aerodrome-outlier")
        .expect("aerodrome stage recorded");
    assert_eq!(
        stage.inputs, 2,
        "1 aerodrome + 1 runway must both be stage inputs"
    );
}

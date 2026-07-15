//! Negative fixtures for disposition semantics: a disposition must name a
//! real, uniquely resolvable input record, never duplicate, never contradict
//! output lineage, and every input record must have a recorded fate.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::source_binding::verify_resigned;
use super::*;

use crate::provenance::{Disposition, RecordDisposition};

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
fn a_dropped_fate_contradicting_output_lineage_fails() {
    // A Clipped/RejectedOutlier/NoContribution fate and output lineage are
    // mutually exclusive claims about one record: fabricating a dropped fate
    // for a record the output draws from must fail even after a re-sign.
    let mut artifact = built();
    let drawn = artifact.provenance.records[0].sources[0];
    artifact.provenance.dispositions.push(RecordDisposition {
        source: drawn,
        disposition: Disposition::Clipped,
    });
    let result = verify_resigned(&mut artifact);
    assert!(
        matches!(result, Err(VerifyError::DispositionInvalid { .. })),
        "a dropped fate contradicting lineage must fail: {result:?}"
    );
}

#[test]
fn deleting_a_records_only_disposition_fails_the_full_verifier() {
    // A clipped obstacle's only trace is its disposition; deleting it leaves
    // an input record with no recorded fate, which the full verifier must
    // refuse rather than let an input silently vanish.
    let mut dataset = fixtures::dataset();
    dataset.obstacles.push(crate::source::Obstacle {
        lat_deg: 0.0,
        lon_deg: 0.0,
        height_m: 50.0,
        kind: crate::source::ObstacleKind::Tower,
        source: SourceRecordRef::obstacle(fixtures::OBSTACLE_SRC, 1),
    });
    let mut artifact = build_package(&fixtures::config(), &dataset).expect("build");
    let clipped = SourceRecordRef::obstacle(fixtures::OBSTACLE_SRC, 1);
    let before = artifact.provenance.dispositions.len();
    artifact
        .provenance
        .dispositions
        .retain(|d| d.source != clipped);
    assert_eq!(
        artifact.provenance.dispositions.len(),
        before - 1,
        "the clipped obstacle had exactly one disposition to delete"
    );
    resign_bundle(&mut artifact);
    let config = fixtures::config();
    let key = SigningKey::from_bytes(&config.signing.signing_seed);
    let trust = TrustRoot::new(vec![TrustAnchor {
        key_id: config.signing.key_id,
        public_key: key.verifying_key().to_bytes(),
    }]);
    let result = verify_artifact_with_sources(
        &artifact,
        &dataset,
        &trust,
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    );
    assert!(
        matches!(
            result,
            Err(VerifyError::UnrecordedSourceFate { source_id })
                if source_id == fixtures::OBSTACLE_SRC.0
        ),
        "an input record with no recorded fate must fail: {result:?}"
    );
}

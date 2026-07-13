//! Tests for output-provenance binding and checking.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::activation::PackageStore;
use crate::error::DbUnavailable;
use crate::fixtures;
use crate::identity::{DatasetId, PackageVersion};
use crate::verify::UsePolicy;

fn candidate(version: PackageVersion) -> crate::tile::CandidatePackage {
    fixtures::candidate_with(DatasetId(1), version, true)
}

/// Acceptance: the active-database id is bound into emitted output and checked
/// against the active package. Output is stamped only when the database is
/// available, the stamp carries the exact active id, and a stamp from a retired
/// package is rejected.
#[test]
fn rendered_output_provenance_bound_and_checked() {
    let trust = fixtures::trust_root();
    let inside = fixtures::position(40.5, -74.5);
    let mut store = PackageStore::new();

    // With no active package there is no output to attribute.
    assert_eq!(
        store.render_stamp(fixtures::NOW, &inside),
        Err(DbUnavailable::NoPackage)
    );

    let id1 = store
        .stage_and_activate(
            &candidate(PackageVersion::new(1, 0, 0)),
            &trust,
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v1 activates");

    // The stamp carries the active id and verifies against the active package.
    let stamp = store.render_stamp(fixtures::NOW, &inside).expect("stamp");
    assert_eq!(stamp.active_db(), id1);
    assert_eq!(store.verify_output_provenance(&stamp), Ok(()));

    // Output outside coverage is not stamped (never attributed to an
    // unavailable database).
    assert_eq!(
        store.render_stamp(fixtures::NOW, &fixtures::position(0.0, 0.0)),
        Err(DbUnavailable::Coverage)
    );

    // After activating a new package, the earlier stamp no longer matches the
    // active database and is refused.
    let id2 = store
        .stage_and_activate(
            &candidate(PackageVersion::new(2, 0, 0)),
            &trust,
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v2 activates");
    assert_ne!(id1, id2);
    assert_eq!(
        store.verify_output_provenance(&stamp),
        Err(DbUnavailable::ProvenanceMismatch)
    );
}

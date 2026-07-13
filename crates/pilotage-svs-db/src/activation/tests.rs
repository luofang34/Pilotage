//! Tests for atomic activation, diagnostics, and coverage availability.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_geo::AvailabilityReason;

use super::PackageStore;
use crate::error::DbUnavailable;
use crate::fixtures;
use crate::identity::{DatasetId, PackageVersion};
use crate::verify::{UsePolicy, verify_package};

fn v1() -> crate::tile::CandidatePackage {
    fixtures::candidate_with(DatasetId(1), PackageVersion::new(1, 0, 0), true)
}

fn v2() -> crate::tile::CandidatePackage {
    fixtures::candidate_with(DatasetId(1), PackageVersion::new(2, 0, 0), true)
}

fn tampered(mut candidate: crate::tile::CandidatePackage) -> crate::tile::CandidatePackage {
    candidate.tiles[0].bytes[0] ^= 0x01;
    candidate
}

/// Acceptance: an interrupted update yields the complete prior package or no
/// package, never a partial or mixed-version mix. Verification is pure and the
/// swap is a single assignment, so a failure never reaches the swap and a
/// verified-but-not-yet-installed token leaves the prior package intact.
#[test]
fn interrupted_update_yields_prior_or_no_package() {
    let trust = fixtures::trust_root();

    // A failed update leaves the prior complete package active.
    let mut store = PackageStore::new();
    let id1 = store
        .stage_and_activate(&v1(), &trust, fixtures::NOW, UsePolicy::SimulatorPermitted)
        .expect("v1 activates");
    let err = store
        .stage_and_activate(
            &tampered(v2()),
            &trust,
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect_err("tampered update fails");
    assert_eq!(err.to_availability_reason(), AvailabilityReason::Database);
    assert_eq!(store.active_id(), Some(id1), "prior package must be intact");

    // A failed first update leaves no package, never a partial one.
    let mut empty = PackageStore::new();
    assert!(
        empty
            .stage_and_activate(
                &tampered(v1()),
                &trust,
                fixtures::NOW,
                UsePolicy::SimulatorPermitted,
            )
            .is_err()
    );
    assert_eq!(empty.active_id(), None, "no partial package on failure");

    // Two-phase: a verified token is held but not yet installed — the swap is
    // the only mutation, so the prior package remains active until it happens.
    let token = verify_package(
        &v2(),
        &trust,
        fixtures::NOW,
        store.active_id(),
        UsePolicy::SimulatorPermitted,
    )
    .expect("v2 verifies");
    assert_eq!(store.active_id(), Some(id1), "prior intact before the swap");
    let id2 = store.activate(token);
    assert_eq!(
        store.active_id(),
        Some(id2),
        "swap installed the new package"
    );
    assert_ne!(id1, id2);
}

/// Acceptance: the active-database id is carried into diagnostics and is
/// available as an accessor for rendered output.
#[test]
fn active_database_id_carried_into_diagnostics() {
    let mut store = PackageStore::new();
    assert_eq!(store.active_id(), None);
    assert_eq!(store.diagnostic_line(), "no active database");

    let id = store
        .stage_and_activate(
            &v1(),
            &fixtures::trust_root(),
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v1 activates");
    assert_eq!(store.active_id(), Some(id));
    assert_eq!(store.active().expect("active").id(), id);

    let line = store.diagnostic_line();
    assert!(
        line.contains(&format!("{id}")),
        "diagnostic names the id: {line}"
    );
    // A simulator package renders its marker so it can never pass as operational.
    assert!(format!("{id}").ends_with("SIM"));
}

#[test]
fn a_position_inside_coverage_returns_the_active_id() {
    let mut store = PackageStore::new();
    let id = store
        .stage_and_activate(
            &v1(),
            &fixtures::trust_root(),
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v1 activates");
    let inside = fixtures::position(40.5, -74.5);
    assert_eq!(store.availability_for_position(&inside), Ok(id));
}

#[test]
fn a_position_outside_coverage_is_a_coverage_exit() {
    let mut store = PackageStore::new();
    store
        .stage_and_activate(
            &v1(),
            &fixtures::trust_root(),
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v1 activates");
    let outside = fixtures::position(0.0, 0.0);
    let reason = store
        .availability_for_position(&outside)
        .expect_err("outside coverage");
    assert_eq!(reason, DbUnavailable::Coverage);
    assert_eq!(
        reason.to_availability_reason(),
        AvailabilityReason::Coverage
    );
}

#[test]
fn no_active_package_is_a_database_fault() {
    let store = PackageStore::new();
    let reason = store
        .availability_for_position(&fixtures::position(40.5, -74.5))
        .expect_err("no package");
    assert_eq!(reason, DbUnavailable::NoPackage);
    assert_eq!(
        reason.to_availability_reason(),
        AvailabilityReason::Database
    );
}

#[test]
fn activation_replaces_the_prior_package() {
    let mut store = PackageStore::new();
    let id1 = store
        .stage_and_activate(
            &v1(),
            &fixtures::trust_root(),
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v1 activates");
    let id2 = store
        .stage_and_activate(
            &v2(),
            &fixtures::trust_root(),
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v2 activates");
    assert_ne!(id1, id2);
    assert_eq!(store.active_id(), Some(id2));
    assert_eq!(id2.version, PackageVersion::new(2, 0, 0));
}

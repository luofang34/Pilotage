//! Tests for atomic activation, diagnostics, and coverage availability.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_geo::AvailabilityReason;

use super::{ActivePackage, PackageStore};
use crate::error::{DbError, DbUnavailable};
use crate::fixtures;
use crate::identity::{DatasetId, DayNumber, PackageVersion};
use crate::tile::CandidatePackage;
use crate::verify::UsePolicy;

/// Uniform tile-payload tag for version 1, so v1 and v2 content is disjoint.
const TAG_V1: u8 = 0x11;
/// Uniform tile-payload tag for version 2.
const TAG_V2: u8 = 0x22;

fn v1() -> CandidatePackage {
    fixtures::candidate_with(DatasetId(1), PackageVersion::new(1, 0, 0), true)
}

fn v2() -> CandidatePackage {
    fixtures::candidate_with(DatasetId(1), PackageVersion::new(2, 0, 0), true)
}

fn v1_tagged() -> CandidatePackage {
    fixtures::candidate_tagged(DatasetId(1), PackageVersion::new(1, 0, 0), TAG_V1)
}

fn v2_tagged() -> CandidatePackage {
    fixtures::candidate_tagged(DatasetId(1), PackageVersion::new(2, 0, 0), TAG_V2)
}

fn tampered(mut candidate: CandidatePackage) -> CandidatePackage {
    candidate.tiles[0].bytes[0] ^= 0x01;
    candidate
}

fn all_tiles_have_tag(active: &ActivePackage, tag: u8) -> bool {
    !active.tiles().is_empty()
        && active
            .tiles()
            .iter()
            .all(|t| t.bytes.iter().all(|&b| b == tag))
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
    let token = store
        .verify(&v2(), &trust, fixtures::NOW, UsePolicy::SimulatorPermitted)
        .expect("v2 verifies");
    assert_eq!(store.active_id(), Some(id1), "prior intact before the swap");
    let id2 = store
        .activate(token)
        .expect("swap installs the new package");
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
    assert_eq!(store.availability(fixtures::NOW, &inside), Ok(id));
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
        .availability(fixtures::NOW, &outside)
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
        .availability(fixtures::NOW, &fixtures::position(40.5, -74.5))
        .expect_err("no package");
    assert_eq!(reason, DbUnavailable::NoPackage);
    assert_eq!(
        reason.to_availability_reason(),
        AvailabilityReason::Database
    );
}

/// Acceptance (data level): a torn/interrupted update yields the complete prior
/// tile content or the complete new tile content, never a mix. v1 and v2 carry
/// disjoint tile payloads, so any residual bytes from the other version would be
/// visible.
#[test]
fn interrupted_update_preserves_complete_content_never_a_mix() {
    let trust = fixtures::trust_root();
    let mut store = PackageStore::new();
    store
        .stage_and_activate(
            &v1_tagged(),
            &trust,
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v1 activates");
    assert!(all_tiles_have_tag(store.active().expect("active"), TAG_V1));

    // A torn update (tampered v2) fails; the complete v1 content stays, with no
    // v2 bytes bleeding in.
    store
        .stage_and_activate(
            &tampered(v2_tagged()),
            &trust,
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect_err("tampered v2 fails");
    let active = store.active().expect("active");
    assert!(all_tiles_have_tag(active, TAG_V1), "prior content intact");
    assert!(
        !active.tiles().iter().any(|t| t.bytes.contains(&TAG_V2)),
        "no torn v2 content mixed in"
    );

    // A verified token held but not yet installed leaves the complete v1 content
    // in place; the swap then installs the complete v2 content with no v1 bytes.
    let token = store
        .verify(
            &v2_tagged(),
            &trust,
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v2 verifies");
    assert!(
        all_tiles_have_tag(store.active().expect("active"), TAG_V1),
        "still complete v1 before the swap"
    );
    store.activate(token).expect("swap installs v2");
    let active = store.active().expect("active");
    assert!(
        all_tiles_have_tag(active, TAG_V2),
        "complete v2 content after swap"
    );
    assert!(
        !active.tiles().iter().any(|t| t.bytes.contains(&TAG_V1)),
        "no residual v1 content"
    );
}

/// Acceptance: currency is re-checked at use time. A package current at
/// activation fails closed once the query time is outside its effectivity/expiry
/// window, rather than being served indefinitely.
#[test]
fn currency_is_rechecked_at_use_time() {
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

    assert_eq!(store.availability(fixtures::NOW, &inside), Ok(id));

    let expired = store
        .availability(DayNumber(250), &inside)
        .expect_err("expired at use time");
    assert_eq!(expired, DbUnavailable::Currency);
    assert_eq!(
        expired.to_availability_reason(),
        AvailabilityReason::Database
    );

    assert_eq!(
        store.availability(DayNumber(50), &inside),
        Err(DbUnavailable::Currency),
        "not yet effective also fails closed"
    );
}

/// Acceptance (fail-closed): a stale verification token cannot roll the active
/// package backward. A token verified against the empty store is refused once a
/// different package has been activated, and the active package (id and content)
/// is left unchanged. The only public activation path is the atomic
/// `stage_and_activate`; the raw token swap is crate-internal and CAS-guarded.
#[test]
fn stale_verification_token_cannot_roll_back_the_active_package() {
    let trust = fixtures::trust_root();
    let mut store = PackageStore::new();

    // Verify v1 against the empty store and hold the token.
    let v1_token = store
        .verify(
            &v1_tagged(),
            &trust,
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v1 verifies");

    // Activate v2 normally.
    let id2 = store
        .stage_and_activate(
            &v2_tagged(),
            &trust,
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("v2 activates");

    // Replaying the stale v1 token is refused; v2 stays fully active.
    let err = store
        .activate(v1_token)
        .expect_err("stale token must be refused");
    assert!(matches!(err, DbError::StaleActivation { .. }));
    assert_eq!(err.to_availability_reason(), AvailabilityReason::Database);
    assert_eq!(store.active_id(), Some(id2), "v2 id remains");
    assert!(
        all_tiles_have_tag(store.active().expect("active"), TAG_V2),
        "v2 content remains; not rolled back to v1"
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

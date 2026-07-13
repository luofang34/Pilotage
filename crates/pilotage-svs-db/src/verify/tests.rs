//! Tests for the verification pipeline and the fail-closed rules.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_geo::AvailabilityReason;

use super::{UsePolicy, verify_package};
use crate::activation::PackageStore;
use crate::error::DbError;
use crate::fixtures;
use crate::identity::{ActiveDbId, DatasetId, DayNumber, PackageVersion, ProviderId};

fn trusted() -> crate::trust::TrustRoot {
    fixtures::trust_root()
}

#[test]
fn a_valid_package_verifies_and_reports_its_id() {
    let candidate = fixtures::candidate();
    let verified = verify_package(
        &candidate,
        &trusted(),
        fixtures::NOW,
        None,
        UsePolicy::SimulatorPermitted,
    )
    .expect("valid package verifies");
    assert_eq!(verified.active_id(), candidate.manifest.active_id());
}

/// Acceptance: a one-byte mutation of any tile fails the tile-root check, and a
/// one-byte mutation of the manifest fails the signature check. Neither is
/// re-signed — a tamperer cannot produce a fresh signature.
#[test]
fn one_byte_mutation_fails_verification() {
    // Tile mutation -> recomputed tile-root disagrees with the declared root.
    let mut tampered_tile = fixtures::candidate();
    tampered_tile.tiles[0].bytes[0] ^= 0x01;
    assert_eq!(
        verify_package(
            &tampered_tile,
            &trusted(),
            fixtures::NOW,
            None,
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::TileRootMismatch)
    );

    // Manifest mutation -> signature no longer verifies.
    let mut tampered_manifest = fixtures::candidate();
    tampered_manifest.manifest.coverage.region.max_lat_deg += 0.001;
    assert_eq!(
        verify_package(
            &tampered_manifest,
            &trusted(),
            fixtures::NOW,
            None,
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::SignatureInvalid)
    );
}

/// Acceptance: the trust-root, rollback, expiry, coverage, and datum rules each
/// fail closed with a deterministic, typed reason. Each rule is asserted by a
/// helper so this anchor stays readable.
#[test]
fn trust_rollback_expiry_coverage_datum_rules_fail_closed() {
    assert_untrusted_root_refused();
    assert_expiry_refused();
    assert_not_yet_effective_refused();
    assert_rollback_blocked();
    assert_wrong_datum_refused();
    assert_coverage_exit_is_coverage_reason();
}

fn assert_untrusted_root_refused() {
    let candidate = fixtures::candidate();
    assert_eq!(
        verify_package(
            &candidate,
            &crate::trust::TrustRoot::new(vec![]),
            fixtures::NOW,
            None,
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::UntrustedRoot {
            key_id: candidate.manifest.signature.key_id,
        })
    );
}

fn assert_expiry_refused() {
    assert!(matches!(
        verify_package(
            &fixtures::candidate(),
            &trusted(),
            DayNumber(250),
            None,
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::Expired { .. })
    ));
}

fn assert_not_yet_effective_refused() {
    assert!(matches!(
        verify_package(
            &fixtures::candidate(),
            &trusted(),
            DayNumber(50),
            None,
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::NotYetEffective { .. })
    ));
}

fn assert_rollback_blocked() {
    let active = ActiveDbId {
        dataset: DatasetId(1),
        provider: ProviderId(0xB2),
        version: PackageVersion::new(2, 0, 0),
        simulation_only: true,
    };
    assert_eq!(
        verify_package(
            &fixtures::candidate(),
            &trusted(),
            fixtures::NOW,
            Some(active),
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::RollbackBlocked {
            active: PackageVersion::new(2, 0, 0),
            candidate: PackageVersion::new(1, 0, 0),
        })
    );
}

fn assert_wrong_datum_refused() {
    let mut wrong = fixtures::candidate();
    wrong.manifest.coverage.horizontal_datum = pilotage_geo::HorizontalDatum::Unknown;
    assert_eq!(
        verify_package(
            &wrong,
            &trusted(),
            fixtures::NOW,
            None,
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::UnknownHorizontalDatum)
    );
}

fn assert_coverage_exit_is_coverage_reason() {
    let mut store = PackageStore::new();
    store
        .stage_and_activate(
            &fixtures::candidate(),
            &trusted(),
            fixtures::NOW,
            UsePolicy::SimulatorPermitted,
        )
        .expect("valid package activates");
    let reason = store
        .availability_for_position(&fixtures::position(0.0, 0.0))
        .expect_err("position is outside coverage");
    assert_eq!(
        reason.to_availability_reason(),
        AvailabilityReason::Coverage
    );
}

/// Acceptance: a `simulation_only` package is never accepted under an
/// operational-use policy, while a non-simulator package is.
#[test]
fn simulation_only_never_accepted_as_operational() {
    let sim = fixtures::candidate();
    assert!(sim.manifest.simulation_only);
    assert_eq!(
        verify_package(
            &sim,
            &trusted(),
            fixtures::NOW,
            None,
            UsePolicy::OperationalRequired,
        ),
        Err(DbError::SimulationOnlyForbidden)
    );

    // The same package is usable in the simulator, still carrying its marker.
    let verified = verify_package(
        &sim,
        &trusted(),
        fixtures::NOW,
        None,
        UsePolicy::SimulatorPermitted,
    )
    .expect("simulator package is usable in the simulator");
    assert!(verified.active_id().simulation_only);

    // A non-simulator package passes the operational policy.
    let operational = fixtures::candidate_with(DatasetId(1), PackageVersion::new(1, 0, 0), false);
    assert!(
        verify_package(
            &operational,
            &trusted(),
            fixtures::NOW,
            None,
            UsePolicy::OperationalRequired,
        )
        .is_ok()
    );
}

#[test]
fn tile_count_mismatch_is_refused() {
    let mut candidate = fixtures::candidate();
    candidate.tiles.pop();
    assert_eq!(
        verify_package(
            &candidate,
            &trusted(),
            fixtures::NOW,
            None,
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::TileCountMismatch {
            declared: 3,
            supplied: 2,
        })
    );
}

#[test]
fn a_duplicate_tile_key_is_refused() {
    let mut candidate = fixtures::candidate();
    let dup = candidate.tiles[0].clone();
    candidate.tiles.push(dup);
    candidate.manifest.tile_count = 4;
    assert!(matches!(
        verify_package(
            &candidate,
            &trusted(),
            fixtures::NOW,
            None,
            UsePolicy::SimulatorPermitted,
        ),
        Err(DbError::DuplicateTile { .. })
    ));
}

#[test]
fn a_legitimately_resigned_change_verifies() {
    let mut candidate = fixtures::candidate();
    candidate.manifest.coverage.region.max_lat_deg += 0.5;
    fixtures::sign(&mut candidate.manifest);
    assert!(
        verify_package(
            &candidate,
            &trusted(),
            fixtures::NOW,
            None,
            UsePolicy::SimulatorPermitted,
        )
        .is_ok()
    );
}

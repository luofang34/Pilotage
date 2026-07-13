//! The full verification pipeline: the only way to mint a [`VerifiedPackage`].
//!
//! [`verify_package`] runs every check a package must pass before it may become
//! active — structure and datum discipline, currency, tile integrity against
//! the Merkle root, signature against the trust root, the simulator-use policy,
//! and the rollback policy — and returns a [`VerifiedPackage`] proof token only
//! when all pass. The function is pure: it reads the candidate, the trust root,
//! the current day, and the active id, and returns a value; it performs no I/O
//! and mutates nothing, so activation can build the new state fully before any
//! swap (see [`crate::activation`]).

use crate::canonical::{manifest_canonical_bytes, tile_canonical_bytes};
use crate::error::DbError;
use crate::identity::{ActiveDbId, DayNumber};
use crate::manifest::{Effectivity, PackageManifest};
use crate::merkle::merkle_root;
use crate::tile::{CandidatePackage, Tile};
use crate::trust::{TrustRoot, verify_signature};

/// Whether a simulator-only package may be activated in this context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsePolicy {
    /// Simulator fixtures are permitted (the SIM default). They still activate
    /// carrying their `simulation_only` marker downstream.
    SimulatorPermitted,
    /// Operational use is required: a `simulation_only` package is refused.
    OperationalRequired,
}

/// A package that passed every verification check. The inner manifest is
/// private and there is no public constructor, so a value of this type is proof
/// that [`verify_package`] accepted the package it describes.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedPackage {
    manifest: PackageManifest,
}

impl VerifiedPackage {
    /// The verified manifest.
    #[must_use]
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    /// The active-database id this package would install.
    #[must_use]
    pub fn active_id(&self) -> ActiveDbId {
        self.manifest.active_id()
    }

    /// Consumes the token, yielding the verified manifest.
    #[must_use]
    pub(crate) fn into_manifest(self) -> PackageManifest {
        self.manifest
    }
}

/// Verifies a candidate package in full, failing closed on the first violated
/// rule. Pure: no I/O, no mutation. `active` is the id of the currently active
/// package (for the rollback policy), `now` the current day (for currency), and
/// `policy` whether a simulator fixture is acceptable here.
///
/// # Errors
///
/// The [`DbError`] naming the first failed check.
pub fn verify_package(
    candidate: &CandidatePackage,
    trust: &TrustRoot,
    now: DayNumber,
    active: Option<ActiveDbId>,
    policy: UsePolicy,
) -> Result<VerifiedPackage, DbError> {
    let manifest = &candidate.manifest;
    manifest.validate_structure()?;
    check_currency(&manifest.effectivity, now)?;
    verify_tiles(candidate)?;
    let signed_bytes = manifest_canonical_bytes(manifest);
    verify_signature(trust, &manifest.signature, &signed_bytes)?;
    check_use_policy(manifest.simulation_only, policy)?;
    check_rollback(manifest, active)?;
    Ok(VerifiedPackage {
        manifest: manifest.clone(),
    })
}

/// Rejects a package that is not yet effective or has expired.
fn check_currency(effectivity: &Effectivity, now: DayNumber) -> Result<(), DbError> {
    if effectivity.is_before_effective(now) {
        return Err(DbError::NotYetEffective {
            now,
            effective: effectivity.effective,
        });
    }
    if effectivity.is_after_expiry(now) {
        return Err(DbError::Expired {
            now,
            expiry: effectivity.expiry,
        });
    }
    Ok(())
}

/// Verifies the tiles supplied match the manifest: exact count, no duplicate
/// keys, and a recomputed tile-root that equals the declared one.
fn verify_tiles(candidate: &CandidatePackage) -> Result<(), DbError> {
    let manifest = &candidate.manifest;
    let supplied = candidate.tiles.len();
    if supplied as u64 != u64::from(manifest.tile_count) {
        return Err(DbError::TileCountMismatch {
            declared: manifest.tile_count,
            supplied: supplied as u32,
        });
    }
    let leaves = tile_leaves_sorted(&candidate.tiles)?;
    if merkle_root(&leaves) != manifest.tile_root {
        return Err(DbError::TileRootMismatch);
    }
    Ok(())
}

/// The tiles' leaf hashes in canonical (key-sorted) order, rejecting a
/// duplicate key so the tile set is unambiguous.
fn tile_leaves_sorted(tiles: &[Tile]) -> Result<Vec<[u8; 32]>, DbError> {
    let mut entries: Vec<(crate::tile::TileKey, [u8; 32])> = tiles
        .iter()
        .map(|tile| {
            (
                tile.key,
                crate::merkle::tile_leaf_hash(&tile_canonical_bytes(tile)),
            )
        })
        .collect();
    entries.sort_by_key(|entry| entry.0);
    for pair in entries.windows(2) {
        if pair[0].0 == pair[1].0 {
            let key = pair[0].0;
            return Err(DbError::DuplicateTile {
                class: key.class,
                lat_index: key.tile.lat_index,
                lon_index: key.tile.lon_index,
            });
        }
    }
    Ok(entries.into_iter().map(|(_, leaf)| leaf).collect())
}

/// Enforces the simulator-use policy.
fn check_use_policy(simulation_only: bool, policy: UsePolicy) -> Result<(), DbError> {
    if simulation_only && policy == UsePolicy::OperationalRequired {
        return Err(DbError::SimulationOnlyForbidden);
    }
    Ok(())
}

/// Rejects an out-of-policy rollback: a candidate older than the active package
/// of the same dataset. A different dataset is a different database, not a
/// rollback of this one.
fn check_rollback(manifest: &PackageManifest, active: Option<ActiveDbId>) -> Result<(), DbError> {
    if let Some(active) = active
        && active.dataset == manifest.provenance.dataset
        && manifest.provenance.version < active.version
    {
        return Err(DbError::RollbackBlocked {
            active: active.version,
            candidate: manifest.provenance.version,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;

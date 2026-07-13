//! Signed, provenance-preserving, atomically activated aeronautical database
//! packages for synthetic vision (SVS-02).
//!
//! A [`PackageManifest`] pins a database of terrain, obstacles, aerodromes,
//! runways, and taxiways: where it came from, when it is valid, the region and
//! datum it covers, a [`TileRoot`] hash over every tile, and an Ed25519
//! signature binding all of it to a configured [`TrustRoot`]. [`verify_package`]
//! checks every tile and the whole manifest before a package may become active;
//! [`PackageStore`] activates it as a pure validate-then-swap, so an interrupted
//! update yields the complete prior package or none, never a partial or
//! mixed-version mix. Every failure — missing tile, corruption, signature or
//! trust failure, expiry, coverage exit, incompatible schema, or wrong datum —
//! is a typed refusal that a consumer maps onto
//! [`pilotage_geo::SvsAvailability::Unavailable`], never a plausible-looking
//! scene.
//!
//! The datum vocabulary is consumed from `pilotage-geo`
//! ([`pilotage_geo::HorizontalDatum`], [`pilotage_geo::VerticalDatum`], and
//! their identities), never re-minted here. The airborne/runtime path verifies
//! an already-present package and reads the active id; it never downloads or
//! mutates a database online.
//!
//! # SIM / NOT FOR FLIGHT
//!
//! This is a simulator/engineering contract. Passing these tests does not
//! qualify a data supplier or an aeronautical database, and fixtures marked
//! [`PackageManifest::simulation_only`] are permanently simulation-only and are
//! never accepted as operational databases.

#![forbid(unsafe_code)]

mod activation;
mod canonical;
mod error;
mod feature;
mod identity;
mod manifest;
mod merkle;
mod output;
mod tile;
mod trust;
mod verify;

#[cfg(test)]
mod fixtures;

pub use activation::{ActivePackage, PackageStore};
pub use canonical::{manifest_canonical_bytes, manifest_content_hash, tile_canonical_bytes};
pub use error::{DbError, DbUnavailable};
pub use feature::{FeatureClass, FeatureSet};
pub use identity::{ActiveDbId, DatasetId, DayNumber, PackageVersion, ProviderId};
pub use manifest::{
    Accuracy, ContentSpec, Coverage, CoverageBox, Effectivity, MANIFEST_SCHEMA_VERSION,
    MIN_SUPPORTED_SCHEMA, PackageManifest, ProcessingChain, ProcessingStep, Provenance, Resolution,
    UseRestrictions, schema_is_compatible,
};
pub use merkle::{TileRoot, merkle_root, tile_leaf_hash};
pub use output::RenderStamp;
pub use tile::{CandidatePackage, Tile, TileKey};
pub use trust::{PackageSignature, TrustAnchor, TrustKeyId, TrustRoot, verify_signature};
pub use verify::{UsePolicy, VerifiedPackage, verify_package};

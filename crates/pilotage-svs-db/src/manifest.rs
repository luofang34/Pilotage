//! The versioned package manifest and its compatibility policy.
//!
//! A [`PackageManifest`] is the signed description of a database package:
//! provenance, effectivity, coverage and datum, content, the tile-root hash
//! over every tile, a permanent `simulation_only` flag, and the signature that
//! binds all of it to a trust root. Everything except the signature is part of
//! the canonical bytes the signature is computed over (see [`crate::canonical`]),
//! so no field can be altered without breaking either the signature (for
//! manifest fields) or the tile-root check (for tile bytes).
//!
//! # Schema compatibility policy
//!
//! The schema is versioned by [`MANIFEST_SCHEMA_VERSION`]. This build reads a
//! manifest only when its `schema_version` lies in
//! `[MIN_SUPPORTED_SCHEMA, MANIFEST_SCHEMA_VERSION]`. A **newer** schema is
//! refused rather than partially parsed — an unknown field layout could carry
//! meaning this build would silently drop — and a schema **older** than the
//! minimum is refused rather than upgraded by guesswork. The policy is
//! deliberately closed at both ends: widening it is an explicit code change,
//! never an implicit acceptance.

mod content;
mod coverage;
mod effectivity;
mod provenance;

#[cfg(test)]
mod tests;

pub use content::{Accuracy, ContentSpec};
pub use coverage::{Coverage, CoverageBox, Resolution};
pub use effectivity::Effectivity;
pub use provenance::{ProcessingChain, ProcessingStep, Provenance, UseRestrictions};

use pilotage_geo::{HorizontalDatum, VerticalDatum};

use crate::error::DbError;
use crate::identity::ActiveDbId;
use crate::merkle::TileRoot;
use crate::trust::PackageSignature;

/// The highest manifest schema version this build produces and reads.
pub const MANIFEST_SCHEMA_VERSION: u16 = 1;

/// The lowest manifest schema version this build still reads.
pub const MIN_SUPPORTED_SCHEMA: u16 = 1;

/// Whether `version` is within the compatibility window this build reads.
#[must_use]
pub const fn schema_is_compatible(version: u16) -> bool {
    MIN_SUPPORTED_SCHEMA <= version && version <= MANIFEST_SCHEMA_VERSION
}

/// The signed description of a database package. Construct it directly; it is
/// validated by [`Self::validate_structure`] and, in full, by the verification
/// pipeline before any activation.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageManifest {
    /// The manifest schema version.
    pub schema_version: u16,
    /// Where the package comes from and how it may be used.
    pub provenance: Provenance,
    /// When the package is valid.
    pub effectivity: Effectivity,
    /// The region and datum it covers.
    pub coverage: Coverage,
    /// What it carries and the quality it claims.
    pub content: ContentSpec,
    /// The number of tiles the package must supply.
    pub tile_count: u32,
    /// The Merkle-style root over every tile.
    pub tile_root: TileRoot,
    /// Whether this is a permanently-marked simulator fixture.
    pub simulation_only: bool,
    /// The signature binding the canonical manifest bytes to a trust root.
    pub signature: PackageSignature,
}

impl PackageManifest {
    /// The active-database id this manifest describes, for output and
    /// diagnostics. The id is content-addressed: it carries the hash of the
    /// signed manifest bytes, so distinct content never shares an id.
    #[must_use]
    pub fn active_id(&self) -> ActiveDbId {
        ActiveDbId {
            dataset: self.provenance.dataset,
            provider: self.provenance.provider,
            version: self.provenance.version,
            simulation_only: self.simulation_only,
            content_hash: crate::canonical::manifest_content_hash(self),
        }
    }

    /// Checks the manifest-intrinsic invariants: schema compatibility, a
    /// non-empty feature set, a valid coverage box, ordered effectivity dates,
    /// and datum discipline. Currency, tiles, signature, and rollback are
    /// checked by the full pipeline, not here.
    ///
    /// # Errors
    ///
    /// The [`DbError`] naming the first violated invariant.
    pub fn validate_structure(&self) -> Result<(), DbError> {
        if !schema_is_compatible(self.schema_version) {
            return Err(DbError::IncompatibleManifest {
                version: self.schema_version,
                min: MIN_SUPPORTED_SCHEMA,
                max: MANIFEST_SCHEMA_VERSION,
            });
        }
        if self.content.features.is_empty() {
            return Err(DbError::EmptyFeatureSet);
        }
        if !self.coverage.region.is_valid() {
            return Err(DbError::InvalidCoverage {
                reason: "non-finite or degenerate bounds",
            });
        }
        if !self.effectivity.is_ordered() {
            return Err(DbError::BadEffectivity);
        }
        if self.provenance.restrictions.has_unknown_bits() {
            return Err(DbError::UnknownUseRestriction {
                restrictions: self.provenance.restrictions.bits(),
            });
        }
        self.validate_datum()
    }

    /// Enforces datum discipline on the coverage: the datum must be known and
    /// carry the identity it requires (a realization for NAD83/ITRF, a geoid
    /// for geometric MSL). A wrong or under-declared datum is refused.
    fn validate_datum(&self) -> Result<(), DbError> {
        let c = &self.coverage;
        if c.horizontal_datum == HorizontalDatum::Unknown {
            return Err(DbError::UnknownHorizontalDatum);
        }
        if c.horizontal_datum.needs_realization() && !c.realization.is_declared() {
            return Err(DbError::UndeclaredRealization);
        }
        if c.vertical_datum == VerticalDatum::Unknown {
            return Err(DbError::UnknownVerticalDatum);
        }
        if c.vertical_datum == VerticalDatum::Msl && !c.geoid.is_declared() {
            return Err(DbError::UndeclaredGeoid);
        }
        Ok(())
    }
}

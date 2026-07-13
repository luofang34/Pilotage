//! Typed, fail-closed failure model for database packages.
//!
//! Every [`DbError`] variant is a refusal to activate or to serve, never a
//! repair: a package that cannot be trusted, does not cover the position, is
//! expired, or is an out-of-policy rollback makes synthetic vision unavailable
//! rather than drawing a plausible-looking scene. Each error collapses to a
//! coarse [`DbUnavailable`] category, which a consumer maps onto
//! [`pilotage_geo::AvailabilityReason`] — `Coverage` when the position simply
//! left the covered region, `Database` for every integrity, provenance,
//! currency, and policy failure.

use pilotage_geo::AvailabilityReason;

use crate::feature::FeatureClass;
use crate::identity::{DayNumber, PackageVersion};
use crate::trust::TrustKeyId;

/// Why a database package was refused for activation or for a query. A refusal
/// carries the context its message needs; there is no silent fallback to a
/// partial or stale package.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DbError {
    /// The manifest schema version is outside the range this build reads, so
    /// the package is refused rather than partially parsed.
    #[error("manifest schema version {version} is outside supported [{min}, {max}]")]
    IncompatibleManifest {
        /// The schema version the manifest declared.
        version: u16,
        /// Lowest schema version this build reads.
        min: u16,
        /// Highest schema version this build reads.
        max: u16,
    },
    /// The package declares no feature class, so it covers nothing.
    #[error("package declares an empty feature set")]
    EmptyFeatureSet,
    /// The horizontal datum is unknown to this build.
    #[error("manifest horizontal datum is unknown; coverage has no interpretable frame")]
    UnknownHorizontalDatum,
    /// The vertical datum is unknown to this build.
    #[error("manifest vertical datum is unknown; heights have no interpretable reference")]
    UnknownVerticalDatum,
    /// A realization-bearing horizontal datum declared no realization.
    #[error("manifest horizontal datum requires a declared realization/reference epoch")]
    UndeclaredRealization,
    /// A geometric-MSL vertical datum declared no geoid model.
    #[error("manifest geometric-MSL datum requires a declared geoid model")]
    UndeclaredGeoid,
    /// The coverage bounding box is non-finite or degenerate.
    #[error("coverage box is invalid: {reason}")]
    InvalidCoverage {
        /// What was wrong with the box.
        reason: &'static str,
    },
    /// The effectivity dates are not ordered `release <= effective <= expiry`.
    #[error("effectivity dates are not ordered release<=effective<=expiry")]
    BadEffectivity,
    /// The current day is before the package becomes effective.
    #[error("package not yet effective: day {now} is before effective day {effective}")]
    NotYetEffective {
        /// The current day number.
        now: DayNumber,
        /// The day the package becomes effective.
        effective: DayNumber,
    },
    /// The current day is after the package expires.
    #[error("package expired: day {now} is after expiry day {expiry}")]
    Expired {
        /// The current day number.
        now: DayNumber,
        /// The day the package expires.
        expiry: DayNumber,
    },
    /// The number of tiles supplied does not match the manifest.
    #[error("tile count mismatch: manifest declares {declared}, package supplies {supplied}")]
    TileCountMismatch {
        /// Tiles the manifest declares.
        declared: u32,
        /// Tiles the package actually supplied.
        supplied: u32,
    },
    /// Two tiles share one key, so the package is ambiguous.
    #[error("duplicate tile {class:?} at ({lat_index}, {lon_index})")]
    DuplicateTile {
        /// The feature class of the duplicated tile.
        class: FeatureClass,
        /// Tile latitude index.
        lat_index: i32,
        /// Tile longitude index.
        lon_index: i32,
    },
    /// The recomputed tile-root hash does not match the manifest, so at least
    /// one tile or the declared root was mutated.
    #[error("tile-root hash mismatch; a tile or the manifest root was mutated")]
    TileRootMismatch,
    /// The signing key id is not in the configured trust root.
    #[error("signing key {key_id} is not a configured trust root")]
    UntrustedRoot {
        /// The key id the manifest named.
        key_id: TrustKeyId,
    },
    /// The manifest signature did not verify against the trust root's key.
    #[error("manifest signature failed verification against the trust root")]
    SignatureInvalid,
    /// The candidate is an older version of the active dataset (out-of-policy
    /// rollback).
    #[error("rollback blocked: candidate {candidate} is older than active {active}")]
    RollbackBlocked {
        /// The active package version.
        active: PackageVersion,
        /// The (older) candidate version.
        candidate: PackageVersion,
    },
    /// The package is a permanently-marked simulator fixture and operational
    /// use was required.
    #[error("package is simulation_only and cannot be accepted as operational")]
    SimulationOnlyForbidden,
}

impl DbError {
    /// The coarse availability category this refusal collapses to.
    #[must_use]
    pub const fn category(&self) -> DbUnavailable {
        match self {
            Self::TileCountMismatch { .. } | Self::DuplicateTile { .. } => {
                DbUnavailable::MissingData
            }
            Self::TileRootMismatch => DbUnavailable::Corrupt,
            Self::UntrustedRoot { .. } | Self::SignatureInvalid => DbUnavailable::Signature,
            Self::NotYetEffective { .. } | Self::Expired { .. } => DbUnavailable::Currency,
            Self::IncompatibleManifest { .. } => DbUnavailable::Incompatible,
            Self::UnknownHorizontalDatum
            | Self::UnknownVerticalDatum
            | Self::UndeclaredRealization
            | Self::UndeclaredGeoid => DbUnavailable::Datum,
            Self::RollbackBlocked { .. } => DbUnavailable::Rollback,
            Self::SimulationOnlyForbidden => DbUnavailable::SimulationOnly,
            Self::EmptyFeatureSet | Self::InvalidCoverage { .. } | Self::BadEffectivity => {
                DbUnavailable::Malformed
            }
        }
    }

    /// The [`pilotage_geo::AvailabilityReason`] a consumer surfaces for this
    /// refusal.
    #[must_use]
    pub const fn to_availability_reason(&self) -> AvailabilityReason {
        self.category().to_availability_reason()
    }
}

/// The coarse reason synthetic vision is unavailable from the database's point
/// of view — the typed value a consumer maps onto
/// [`pilotage_geo::AvailabilityReason`]. Everything but a plain coverage exit
/// resolves to `Database`, because it is the database (its integrity,
/// provenance, currency, policy, or presence) that is at fault, not the
/// aircraft's position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbUnavailable {
    /// No package is active.
    NoPackage,
    /// A required tile is missing, duplicated, or miscounted.
    MissingData,
    /// A tile's content did not match its hash (corruption or tampering).
    Corrupt,
    /// The signature or trust root failed.
    Signature,
    /// The package is not yet effective or has expired.
    Currency,
    /// The manifest schema is incompatible with this build.
    Incompatible,
    /// The datum discipline was violated (unknown or undeclared datum).
    Datum,
    /// The activation would be an out-of-policy rollback.
    Rollback,
    /// The package is a simulator fixture and cannot be used operationally.
    SimulationOnly,
    /// The manifest is structurally malformed (empty, degenerate, misordered).
    Malformed,
    /// The query position lies outside the package's covered region.
    Coverage,
}

impl DbUnavailable {
    /// Maps to the geospatial availability reason a consumer reports. Only a
    /// coverage exit is `Coverage`; every other cause is a `Database` fault.
    #[must_use]
    pub const fn to_availability_reason(self) -> AvailabilityReason {
        match self {
            Self::Coverage => AvailabilityReason::Coverage,
            _ => AvailabilityReason::Database,
        }
    }
}

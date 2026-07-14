//! The fixed, deterministic build configuration.
//!
//! Every parameter that shapes the output is stated here, up front: the target
//! datum, the tile size, the interpolation grid, the outlier and hole policies,
//! the effectivity window, and the signing key seed. Nothing is read from a wall
//! clock or a random source, so a fixed configuration and a fixed source dataset
//! reproduce a byte-identical package. The signing key seed lives in the
//! configuration (supplied by the offline publisher), never generated at build
//! time.

#[cfg(test)]
mod tests;

use pilotage_geo::{
    DatumRealizationId, GeoidModelId, HorizontalDatum, IntegrityLevel, VerticalDatum,
};
use pilotage_svs_db::{
    CoverageBox, DatasetId, Effectivity, PackageVersion, ProviderId, TrustKeyId, UseRestrictions,
};

use crate::error::BuildError;

/// The identity, effectivity, and use policy of the package being built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PackageIdentity {
    /// The dataset this package is a release of.
    pub dataset: DatasetId,
    /// The provider producing it.
    pub provider: ProviderId,
    /// The package version.
    pub version: PackageVersion,
    /// The effectivity window (fixed day numbers, never a clock read).
    pub effectivity: Effectivity,
    /// Whether the package is a permanently-marked simulator fixture.
    pub simulation_only: bool,
    /// Restrictions the publisher imposes regardless of source licenses; the
    /// emitted package carries the union of these and every source's license.
    pub base_restrictions: UseRestrictions,
}

/// The datum every output coordinate and height is converted to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetDatum {
    /// The target horizontal datum.
    pub horizontal: HorizontalDatum,
    /// The target horizontal-datum realization; `UNDECLARED` when none needed.
    pub realization: DatumRealizationId,
    /// The target vertical datum.
    pub vertical: VerticalDatum,
    /// The target geoid model; `UNDECLARED` unless the vertical datum needs one.
    pub geoid: GeoidModelId,
}

/// The chain's numeric policies: grid, tiling, outlier rejection, hole filling,
/// and obstacle merging.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChainParams {
    /// Tile size in degrees (positive, finite).
    pub tile_deg: f64,
    /// Output terrain grid spacing in degrees (positive, finite).
    pub post_spacing_deg: f64,
    /// The recorded post spacing in millimeters, for the manifest resolution.
    pub post_spacing_mm: u32,
    /// Lowest plausible terrain/aerodrome elevation, meters; below is an outlier.
    pub elevation_min_m: f64,
    /// Highest plausible terrain/aerodrome elevation, meters; above is an
    /// outlier.
    pub elevation_max_m: f64,
    /// Highest plausible obstacle AGL height, meters; above (or non-positive) is
    /// an outlier.
    pub max_obstacle_height_m: f64,
    /// The largest void span (in source nodes) a hole may be interpolated
    /// across; a wider void stays a void rather than being invented.
    pub max_hole_span: u32,
    /// Obstacles of the same kind within this angular distance (degrees) merge.
    pub merge_tolerance_deg: f64,
    /// The integrity level the package declares for its content.
    pub integrity: IntegrityLevel,
}

/// The signing identity: the trust key id and the Ed25519 seed to sign with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigningConfig {
    /// The trust key id the signature declares.
    pub key_id: TrustKeyId,
    /// The 32-byte Ed25519 signing-key seed (fixed; never generated at build
    /// time).
    pub signing_seed: [u8; 32],
}

/// The complete, deterministic build configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildConfig {
    /// Package identity and effectivity.
    pub identity: PackageIdentity,
    /// The target coverage region.
    pub coverage: CoverageBox,
    /// The datum every output is converted to.
    pub target: TargetDatum,
    /// The chain's numeric policies.
    pub params: ChainParams,
    /// The signing identity.
    pub signing: SigningConfig,
}

impl BuildConfig {
    /// Checks the configuration is internally consistent and deterministic to
    /// run. Called before any stage so a bad parameter aborts before work.
    ///
    /// # Errors
    ///
    /// [`BuildError::InvalidConfig`] naming the first violated invariant.
    pub fn validate(&self) -> Result<(), BuildError> {
        let p = &self.params;
        if !(p.tile_deg.is_finite() && p.tile_deg > 0.0) {
            return Err(BuildError::InvalidConfig {
                reason: "tile_deg must be positive and finite",
            });
        }
        if !(p.post_spacing_deg.is_finite() && p.post_spacing_deg > 0.0) {
            return Err(BuildError::InvalidConfig {
                reason: "post_spacing_deg must be positive and finite",
            });
        }
        if !self.coverage.is_valid() {
            return Err(BuildError::InvalidConfig {
                reason: "coverage box is non-finite or degenerate",
            });
        }
        if !self.identity.effectivity.is_ordered() {
            return Err(BuildError::InvalidConfig {
                reason: "effectivity is not ordered release<=effective<=expiry",
            });
        }
        if !(p.elevation_min_m.is_finite()
            && p.elevation_max_m.is_finite()
            && p.elevation_min_m < p.elevation_max_m)
        {
            return Err(BuildError::InvalidConfig {
                reason: "elevation bounds must be finite with min < max",
            });
        }
        if !(p.max_obstacle_height_m.is_finite() && p.max_obstacle_height_m > 0.0) {
            return Err(BuildError::InvalidConfig {
                reason: "max_obstacle_height_m must be positive and finite",
            });
        }
        if !(p.merge_tolerance_deg.is_finite() && p.merge_tolerance_deg >= 0.0) {
            return Err(BuildError::InvalidConfig {
                reason: "merge_tolerance_deg must be non-negative and finite",
            });
        }
        Ok(())
    }
}

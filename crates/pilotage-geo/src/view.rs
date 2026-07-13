//! The projection view: a reference to one validated calibration plus the
//! render-time projection and clip policy.
//!
//! There is exactly one authoritative camera model in the program — the
//! versioned, hashed calibration artifact (the SVS-01 sibling calibration
//! contract, ADR-0021). This crate does **not** re-mint a second camera model:
//! intrinsics, distortion, principal point, viewport, extrinsics, boresight,
//! design eye, and the alignment-error bound all live in that artifact. A
//! [`ProjectionView`] only *references* the accepted calibration by identity and
//! content hash, carries its published alignment bound, and adds the render-time
//! policy (projection kind and payload, near/far, minification). A consumer
//! resolves the reference against a validated artifact to obtain the geometry;
//! the field of view is a property of that resolved calibration, never stored
//! here.

use crate::error::GeoError;

/// The one canonical calibration identity, owned by the dependency-free `no_std`
/// `pilotage-calibration-id` leaf and re-exported here. A geo projection
/// reference and the `std` calibration artifact it points at are the *same*
/// type over one identity space, not two mirrored `u32`s a lossy conversion
/// could drift apart.
pub use pilotage_calibration_id::CalibrationId;

/// Compile-time proof that geo's `CalibrationId` *is* the leaf's, not a second
/// definition: a leaf value must be usable here with no conversion. If this
/// re-export is ever replaced by a local type, the coercion fails the build.
const _: fn(CalibrationId) -> pilotage_calibration_id::CalibrationId = |id| id;

/// A reference to one accepted, validated calibration artifact — identity and
/// content hash only. The view is meaningless without it, and it deliberately
/// carries **no** geometry or error bound: the alignment bound, principal
/// point, distortion, boresight, design eye, and lifecycle all belong to the
/// artifact and are obtained by *resolving* this reference against a verified
/// artifact (in the `std` calibration contract). A producer cannot write an
/// alignment bound onto the wire here, so it cannot understate one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalibrationRef {
    /// Accepted-calibration identity (must be referenced).
    pub calibration_id: CalibrationId,
    /// The artifact's recorded content hash (must not be all-zero); a resolver
    /// admits the reference only if a verified artifact with this id hashes to
    /// exactly this value.
    pub content_hash: [u8; 32],
}

impl CalibrationRef {
    /// Validates the reference is well-formed: a referenced id and a non-zero
    /// content hash. Whether the referenced artifact exists, is unexpired, and
    /// matches the camera is a *resolution* concern the consumer settles
    /// against a verified artifact — this contract only guarantees the
    /// reference names something.
    ///
    /// # Errors
    ///
    /// [`GeoError::IncompleteCalibrationReference`] when id or hash is missing.
    pub fn validate(&self) -> Result<(), GeoError> {
        let hash_declared = self.content_hash.iter().any(|&b| b != 0);
        if !self.calibration_id.is_referenced() || !hash_declared {
            return Err(GeoError::IncompleteCalibrationReference);
        }
        Ok(())
    }
}

/// The projection a renderer must apply, with the payload each kind needs. A
/// perspective view derives its field of view from the referenced calibration's
/// focal lengths and viewport; an orthographic view is defined by metric
/// extents, because a focal-derived field of view is not an orthographic
/// invariant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    /// Perspective projection; the field of view derives from the referenced
    /// calibration (focal lengths + viewport), never stored here.
    Perspective,
    /// Orthographic projection defined by its metric extents across the
    /// viewport, meters. Both extents must be finite and positive.
    Orthographic {
        /// Metric extent across the viewport width, meters.
        extent_x_m: f64,
        /// Metric extent across the viewport height, meters.
        extent_y_m: f64,
    },
}

impl Projection {
    /// The wire discriminant for the projection kind.
    #[must_use]
    pub const fn kind_u8(self) -> u8 {
        match self {
            Self::Perspective => 0,
            Self::Orthographic { .. } => 1,
        }
    }
}

/// How a renderer samples terrain/tiles under minification (distant detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MinificationPolicy {
    /// Nearest-sample (no filtering).
    Nearest = 0,
    /// Bilinear within one level.
    Bilinear = 1,
    /// Trilinear across mip levels.
    Trilinear = 2,
}

impl MinificationPolicy {
    /// Wire encoding.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
    /// Decodes the wire byte, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Nearest),
            1 => Some(Self::Bilinear),
            2 => Some(Self::Trilinear),
            _ => None,
        }
    }
}

/// The near/far clip policy, in meters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NearFarPolicy {
    /// Near clip distance, meters (must be `> 0`).
    pub near_m: f64,
    /// Far clip distance, meters (must be `> near`).
    pub far_m: f64,
}

impl NearFarPolicy {
    /// Whether a depth (meters along the optical axis) is within the clip
    /// range, inclusive of both planes — the projection-boundary predicate.
    #[must_use]
    pub fn contains_depth(&self, depth_m: f64) -> bool {
        depth_m >= self.near_m && depth_m <= self.far_m
    }
}

/// The projection view: a reference to the one validated calibration plus the
/// render-time projection and clip policy. It holds no camera model of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectionView {
    /// Reference to the accepted, validated calibration artifact.
    pub calibration: CalibrationRef,
    /// Projection kind and payload.
    pub projection: Projection,
    /// Near/far clip policy.
    pub near_far: NearFarPolicy,
    /// Minification sampling policy.
    pub minification: MinificationPolicy,
}

impl ProjectionView {
    /// Validates the view, failing closed on an incomplete calibration
    /// reference, an invalid clip policy, or an orthographic projection without
    /// positive finite extents.
    ///
    /// # Errors
    ///
    /// A [`GeoError`] describing the first violation.
    pub fn validate(&self) -> Result<(), GeoError> {
        self.calibration.validate()?;
        if !(self.near_far.near_m.is_finite() && self.near_far.far_m.is_finite())
            || self.near_far.near_m <= 0.0
            || self.near_far.far_m <= self.near_far.near_m
        {
            return Err(GeoError::InvalidNearFar {
                near_m: self.near_far.near_m,
                far_m: self.near_far.far_m,
            });
        }
        if let Projection::Orthographic {
            extent_x_m,
            extent_y_m,
        } = self.projection
            && (!(extent_x_m.is_finite() && extent_y_m.is_finite())
                || extent_x_m <= 0.0
                || extent_y_m <= 0.0)
        {
            return Err(GeoError::InvalidOrthographicExtent {
                extent_x_m,
                extent_y_m,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

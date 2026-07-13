//! The one canonical calibration identity for the program.
//!
//! A camera calibration artifact and a synthetic-vision projection reference
//! both need to name *which* calibration applies, and they must agree on one
//! identity space — a reference that points at an artifact by a different-width
//! or separately-defined id could silently drift. This crate is that single
//! definition: a `no_std`, dependency-free leaf both the `std` calibration
//! contract (`pilotage-adapter-api`) and the geospatial contract
//! (`pilotage-geo`) depend on and re-export, so neither owns the id relative to
//! the other and neither pulls the other's whole contract to name a `u32`.
//!
//! SIM / NOT FOR FLIGHT.

#![no_std]
#![forbid(unsafe_code)]

/// The identity of one camera calibration (intrinsics/extrinsics) artifact.
///
/// `0` is the [`CalibrationId::NONE`] sentinel: no calibration identity is
/// referenced, and a consumer must treat that as "calibration unavailable"
/// rather than assume a default. Every other value names one artifact; the
/// artifact's geometry, lifecycle, content hash, and error budget live in the
/// calibration contract, never here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CalibrationId(pub u32);

impl CalibrationId {
    /// The sentinel meaning no calibration identity is referenced.
    pub const NONE: Self = Self(0);

    /// Whether a calibration identity is actually referenced (not [`Self::NONE`]).
    #[must_use]
    pub const fn is_referenced(self) -> bool {
        self.0 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::CalibrationId;

    #[test]
    fn none_is_the_zero_sentinel_and_is_not_referenced() {
        assert_eq!(CalibrationId::NONE, CalibrationId(0));
        assert!(!CalibrationId::NONE.is_referenced());
        assert!(CalibrationId(1).is_referenced());
    }
}

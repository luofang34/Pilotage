//! Content specification: which feature classes a package carries and the
//! accuracy and integrity it claims for them.
//!
//! The integrity level is consumed from `pilotage-geo`
//! ([`pilotage_geo::IntegrityLevel`]) rather than re-minted, so a package's
//! declared assurance is expressed in the same vocabulary the availability
//! contract judges an input by.

use pilotage_geo::IntegrityLevel;

use crate::feature::FeatureSet;

/// The declared 1-sigma accuracy of a package's data, split by axis so a
/// horizontal accuracy can never be read as a vertical one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accuracy {
    /// Horizontal 1-sigma accuracy, millimeters.
    pub horizontal_mm: u32,
    /// Vertical 1-sigma accuracy, millimeters.
    pub vertical_mm: u32,
}

/// What a package carries and the quality it claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentSpec {
    /// The feature classes present.
    pub features: FeatureSet,
    /// The declared accuracy.
    pub accuracy: Accuracy,
    /// The declared integrity/assurance level.
    pub integrity: IntegrityLevel,
}

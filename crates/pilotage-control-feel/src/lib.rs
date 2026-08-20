//! Versioned operator control-feel profiles and pure demand shaping.

mod digest;
mod profile;
mod shaper;
mod validation;

pub use digest::{FeelDigest, FeelDigestError};
pub use profile::{
    AxisCurve, AxisDynamics, AxisResponse, DemandEnvelope, DirectDynamics, FeelMode,
    FlightFeelProfile, HoldTransition, NeutralBand, SCHEMA_VERSION,
};
pub use shaper::{
    AxisDemandShaper, DemandPhase, HoldDetector, JerkLimitedAxis, NeutralLatch, ShapedDemand,
};
pub use validation::{ProfileLoadError, ValidatedFlightFeelProfile, ValidationError};

#[cfg(test)]
mod tests;

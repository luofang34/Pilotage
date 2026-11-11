//! Versioned operator control-feel profiles and pure demand shaping.

pub mod airframe;
mod binding;
mod digest;
mod fixed_wing;
mod profile;
mod shaper;
mod validation;

pub use airframe::AirframeLimits;
pub use digest::{FeelDigest, FeelDigestError};
pub use fixed_wing::{FixedWingEnvelope, ValidatedFixedWingEnvelope};
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
pub use binding::{DeviceProfileDigest, FlightControllerDigest, ProfileBindings};

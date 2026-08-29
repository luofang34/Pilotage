//! Simulator plant and hover-trim variation contracts.
//!
//! A plant request changes the simulated aircraft itself: what it carries and
//! where the mass sits. It acts before the run, so it never changes a
//! controller value and never changes the stimulus.

use serde::{Deserialize, Serialize};

use crate::{ValidationError, validation::range};

#[cfg(test)]
mod tests;

const MAX_PAYLOAD_DELTA_KG: f64 = 2_000.0;
const MAX_CG_OFFSET_M: f64 = 2.0;
const MAX_HOVER_RATIO_ERROR: f64 = 0.1;

/// One check for the hover thrust that supports aircraft weight.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HoverThrustExpectation {
    /// Use the ratio from the exact simulator total-mass readback.
    MeasuredWeightRatio,
    /// Compare the measured ratio with an explicit value.
    ExplicitRatio {
        /// The expected hover-thrust ratio.
        ratio: f64,
        /// The maximum permitted absolute error.
        maximum_error: f64,
    },
}

/// One declared simulator plant and hover-trim variation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlantCondition {
    /// Payload mass added to the baseline aircraft in kilograms.
    pub payload_mass_delta_kg: f64,
    /// Longitudinal center-of-gravity offset from the default in meters.
    pub longitudinal_cg_offset_m: f64,
    /// Lateral center-of-gravity offset from the default in meters.
    pub lateral_cg_offset_m: f64,
    /// Check for the hover thrust that supports aircraft weight.
    pub hover_thrust_expectation: HoverThrustExpectation,
}

impl PlantCondition {
    /// Returns the baseline plant declaration.
    #[must_use]
    pub const fn nominal() -> Self {
        Self {
            payload_mass_delta_kg: 0.0,
            longitudinal_cg_offset_m: 0.0,
            lateral_cg_offset_m: 0.0,
            hover_thrust_expectation: HoverThrustExpectation::MeasuredWeightRatio,
        }
    }

    /// Validates the complete plant declaration.
    ///
    /// # Errors
    ///
    /// Returns an error when a mass or center-of-gravity request is outside
    /// its fixed bound, or when the hover-thrust check is outside its bound.
    pub fn validate(self) -> Result<(), ValidationError> {
        range(
            "condition_set.plant.payload_mass_delta_kg",
            self.payload_mass_delta_kg,
            0.0,
            MAX_PAYLOAD_DELTA_KG,
        )?;
        range(
            "condition_set.plant.longitudinal_cg_offset_m",
            self.longitudinal_cg_offset_m,
            -MAX_CG_OFFSET_M,
            MAX_CG_OFFSET_M,
        )?;
        range(
            "condition_set.plant.lateral_cg_offset_m",
            self.lateral_cg_offset_m,
            -MAX_CG_OFFSET_M,
            MAX_CG_OFFSET_M,
        )?;
        self.hover_thrust_expectation.validate()
    }
}

impl HoverThrustExpectation {
    fn validate(self) -> Result<(), ValidationError> {
        match self {
            Self::MeasuredWeightRatio => Ok(()),
            Self::ExplicitRatio {
                ratio,
                maximum_error,
            } => {
                range(
                    "condition_set.plant.hover_thrust_expectation.ratio",
                    ratio,
                    0.5,
                    1.5,
                )?;
                range(
                    "condition_set.plant.hover_thrust_expectation.maximum_error",
                    maximum_error,
                    0.0,
                    MAX_HOVER_RATIO_ERROR,
                )
            }
        }
    }

    /// Returns whether one measured ratio satisfies this check.
    ///
    /// A measured ratio that is not a finite positive number satisfies no
    /// check, so an absent or failed mass readback cannot pass by default.
    #[must_use]
    pub fn accepts(self, measured_ratio: f64) -> bool {
        if !measured_ratio.is_finite() || measured_ratio <= 0.0 {
            return false;
        }
        match self {
            Self::MeasuredWeightRatio => true,
            Self::ExplicitRatio {
                ratio,
                maximum_error,
            } => {
                let rounding = f64::EPSILON * ratio.abs().max(measured_ratio.abs()).max(1.0) * 4.0;
                (measured_ratio - ratio).abs() <= maximum_error + rounding
            }
        }
    }

    /// Returns the explicit ratio and maximum error when they exist.
    #[must_use]
    pub const fn explicit(self) -> Option<(f64, f64)> {
        match self {
            Self::MeasuredWeightRatio => None,
            Self::ExplicitRatio {
                ratio,
                maximum_error,
            } => Some((ratio, maximum_error)),
        }
    }
}

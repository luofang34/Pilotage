//! Controller initialization uncertainty contracts.
//!
//! This input changes the force-domain hover feed-forward value before kernel
//! construction. It must not change estimator state, actuator output bias,
//! plant mass, or center of gravity, and it is never a plant readback
//! expectation.

use serde::{Deserialize, Serialize};

use crate::{BackendCapability, ValidationError};

#[cfg(test)]
mod tests;

const BASIS_POINTS_NOMINAL: u16 = 10_000;
const MIN_HOVER_SCALE_BASIS_POINTS: u16 = 8_000;
const MAX_HOVER_SCALE_BASIS_POINTS: u16 = 12_000;

/// A force-domain hover initialization policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HoverThrustForceInitialization {
    /// Scale the fixed baseline before controller construction.
    ScaleBaseline {
        /// The baseline scale in basis points.
        scale_basis_points: u16,
    },
}

/// Controller initialization inputs for one condition set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerInitializationCondition {
    /// The force-domain hover initialization.
    pub hover_thrust_force: HoverThrustForceInitialization,
}

impl ControllerInitializationCondition {
    /// Returns the nominal controller initialization.
    #[must_use]
    pub const fn nominal() -> Self {
        Self {
            hover_thrust_force: HoverThrustForceInitialization::ScaleBaseline {
                scale_basis_points: BASIS_POINTS_NOMINAL,
            },
        }
    }

    /// Reports whether the hover-force scale is nominal.
    #[must_use]
    pub const fn has_nominal_hover_thrust_force(self) -> bool {
        self.hover_thrust_force.scale_basis_points() == BASIS_POINTS_NOMINAL
    }

    /// Returns the capabilities that this controller initialization needs.
    #[must_use]
    pub fn required_capabilities(self) -> Vec<BackendCapability> {
        if self.has_nominal_hover_thrust_force() {
            Vec::new()
        } else {
            vec![BackendCapability::HoverTrimUncertainty]
        }
    }

    /// Validates the controller initialization.
    ///
    /// # Errors
    ///
    /// Returns an error when the hover-force scale is outside its bound.
    pub fn validate(self) -> Result<(), ValidationError> {
        self.hover_thrust_force.validate()
    }
}

impl HoverThrustForceInitialization {
    /// Returns the configured baseline scale in basis points.
    #[must_use]
    pub const fn scale_basis_points(self) -> u16 {
        match self {
            Self::ScaleBaseline { scale_basis_points } => scale_basis_points,
        }
    }

    /// Scales a baseline force and checks its valid open interval.
    ///
    /// # Errors
    ///
    /// Returns an error when a supplied force is not finite, when the
    /// interval is not ordered, or when the effective force reaches an
    /// interval limit.
    pub fn effective_force(
        self,
        baseline_force: f64,
        minimum_exclusive: f64,
        maximum_exclusive: f64,
    ) -> Result<f64, ValidationError> {
        self.validate()?;
        if !baseline_force.is_finite()
            || !minimum_exclusive.is_finite()
            || !maximum_exclusive.is_finite()
            || minimum_exclusive >= maximum_exclusive
        {
            return Err(ValidationError::InvalidRelation {
                field: "condition_set.controller_initialization.hover_thrust_force".to_owned(),
                relation: "use finite force values and an ordered open interval",
            });
        }
        let effective =
            baseline_force * f64::from(self.scale_basis_points()) / f64::from(BASIS_POINTS_NOMINAL);
        if effective > minimum_exclusive && effective < maximum_exclusive {
            return Ok(effective);
        }
        Err(ValidationError::InvalidRelation {
            field: "condition_set.controller_initialization.hover_thrust_force".to_owned(),
            relation: "stay inside the valid open force interval after scaling",
        })
    }

    fn validate(self) -> Result<(), ValidationError> {
        let actual = self.scale_basis_points();
        if (MIN_HOVER_SCALE_BASIS_POINTS..=MAX_HOVER_SCALE_BASIS_POINTS).contains(&actual) {
            return Ok(());
        }
        Err(ValidationError::OutOfRange {
            field: "condition_set.controller_initialization.hover_thrust_force.scale_basis_points"
                .to_owned(),
            actual: f64::from(actual),
            minimum: f64::from(MIN_HOVER_SCALE_BASIS_POINTS),
            maximum: f64::from(MAX_HOVER_SCALE_BASIS_POINTS),
        })
    }
}

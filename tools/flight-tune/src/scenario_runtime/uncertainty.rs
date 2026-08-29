//! Two-stage admission for deterministic uncertainty conditions.
//!
//! Preparation admits a condition against the known backend declaration,
//! before runtime discovery and before any process starts. Arming repeats the
//! check against the live runtime, so a backend that changed its declaration
//! between preparation and flight cannot reach a run.

use pilotage_trial::{BackendCapability, ConditionSet, HoverEstimatorMode};

use super::{ScenarioRuntime, ScenarioRuntimeError};

#[cfg(test)]
mod tests;

/// What one backend reports about the uncertainty it can execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncertaintyDeclaration {
    capabilities: Vec<BackendCapability>,
    hover_estimator_mode: HoverEstimatorMode,
}

impl UncertaintyDeclaration {
    /// Creates one uncertainty declaration.
    #[must_use]
    pub fn new(
        capabilities: Vec<BackendCapability>,
        hover_estimator_mode: HoverEstimatorMode,
    ) -> Self {
        Self {
            capabilities,
            hover_estimator_mode,
        }
    }

    /// Creates the declaration that a runtime reports now.
    #[must_use]
    pub fn from_runtime(runtime: &dyn ScenarioRuntime) -> Self {
        Self::new(
            runtime.uncertainty_capabilities().to_vec(),
            runtime.hover_estimator_mode(),
        )
    }

    /// Returns the reported uncertainty capabilities.
    #[must_use]
    pub fn capabilities(&self) -> &[BackendCapability] {
        &self.capabilities
    }

    /// Returns the reported hover-estimator mode.
    #[must_use]
    pub const fn hover_estimator_mode(&self) -> HoverEstimatorMode {
        self.hover_estimator_mode
    }

    fn admit(&self, condition: &ConditionSet) -> Result<(), ScenarioRuntimeError> {
        condition
            .validate_capability_report(&self.capabilities, self.hover_estimator_mode)
            .map_err(|source| ScenarioRuntimeError::UnsupportedCondition {
                condition: condition.id.clone(),
                source,
            })
    }
}

/// The known backend declaration that admits a condition before a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionAdmission {
    declared: UncertaintyDeclaration,
}

impl ConditionAdmission {
    /// Creates one admission over the known backend declaration.
    #[must_use]
    pub const fn new(declared: UncertaintyDeclaration) -> Self {
        Self { declared }
    }

    /// Returns the known backend declaration.
    #[must_use]
    pub const fn declared(&self) -> &UncertaintyDeclaration {
        &self.declared
    }

    /// Admits one condition during plan preparation.
    ///
    /// # Errors
    ///
    /// Returns an error when the condition is invalid, or when the known
    /// declaration does not supply an exact required capability.
    pub fn prepare(&self, condition: &ConditionSet) -> Result<(), ScenarioRuntimeError> {
        self.declared.admit(condition)
    }

    /// Admits one condition against the live runtime before arming.
    ///
    /// # Errors
    ///
    /// Returns an error when the live runtime reports a declaration that
    /// differs from the prepared one, or when the live declaration does not
    /// supply an exact required capability.
    pub fn admit_live(
        &self,
        condition: &ConditionSet,
        runtime: &dyn ScenarioRuntime,
    ) -> Result<(), ScenarioRuntimeError> {
        let live = UncertaintyDeclaration::from_runtime(runtime);
        if live.hover_estimator_mode != self.declared.hover_estimator_mode {
            return Err(ScenarioRuntimeError::ChangedHoverEstimatorMode {
                prepared: self.declared.hover_estimator_mode.as_str(),
                live: live.hover_estimator_mode.as_str(),
            });
        }
        if live.capabilities != self.declared.capabilities {
            return Err(ScenarioRuntimeError::ChangedUncertaintyCapabilities {
                condition: condition.id.clone(),
            });
        }
        live.admit(condition)
    }
}

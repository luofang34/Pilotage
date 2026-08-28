//! Caller-supplied clocks, observations, and tick input.

use serde::{Deserialize, Serialize};

use crate::{
    Digest, DirectiveReceipt, EngineInputError, ExecutionTarget, SignalSelector,
    VehicleLifecycleState,
};

/// One absolute wall deadline bound to a mission identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallDeadline {
    /// The mission content digest that owns the deadline.
    pub mission_content_digest: Digest,
    /// The absolute caller wall-clock value when the deadline expires.
    pub expires_at_ns: u64,
}

/// Caller-supplied values that start one engine run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineStart {
    /// The host execution target.
    pub host_target: ExecutionTarget,
    /// The simulator clock at mission admission.
    pub simulator_time_ns: u64,
    /// The wall clock at mission admission.
    pub wall_time_ns: u64,
    /// The identity-bound wall deadline.
    pub wall_deadline: WallDeadline,
}

/// Navigation values in one observation frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationObservation {
    /// The guidance validity, when available.
    pub guidance_valid: Option<bool>,
    /// The active-plan completion state, when available.
    pub plan_complete: Option<bool>,
    /// The observed altitude in meters, when available.
    pub altitude_m: Option<f64>,
}

/// Vehicle values in one observation frame.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleObservation {
    /// The lifecycle state, when available.
    pub lifecycle: Option<VehicleLifecycleState>,
    /// The ground-contact state, when available.
    pub ground_contact: Option<bool>,
    /// The crash state, when available.
    pub crashed: Option<bool>,
    /// The control-link validity, when available.
    pub link_valid: Option<bool>,
    /// The estimator validity, when available.
    pub estimator_valid: Option<bool>,
}

/// One exact scalar signal value in an observation frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedSignal {
    /// The exact signal selector.
    pub selector: SignalSelector,
    /// The observed scalar value.
    pub value: f64,
}

/// The non-clock observations for one engine tick.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionObservation {
    /// Navigation observations.
    pub navigation: NavigationObservation,
    /// Vehicle observations.
    pub vehicle: VehicleObservation,
    /// Exact scalar signal observations.
    pub signals: Vec<ObservedSignal>,
}

/// All caller-supplied input for one engine tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickInput {
    /// The current simulator clock.
    pub simulator_time_ns: u64,
    /// The current wall clock.
    pub wall_time_ns: u64,
    /// The current observation frame.
    pub observation: MissionObservation,
    /// Receipts that arrived before this tick.
    pub receipts: Vec<DirectiveReceipt>,
}

impl MissionObservation {
    pub(crate) fn validate(&self) -> Result<(), EngineInputError> {
        if self
            .navigation
            .altitude_m
            .is_some_and(|value| !value.is_finite())
        {
            return Err(EngineInputError::NonFiniteObservation {
                field: "observation.navigation.altitude_m".to_owned(),
            });
        }
        for (index, signal) in self.signals.iter().enumerate() {
            if !signal.value.is_finite() {
                return Err(EngineInputError::NonFiniteObservation {
                    field: format!("observation.signals[{index}].value"),
                });
            }
            if self.signals[..index]
                .iter()
                .any(|prior| prior.selector == signal.selector)
            {
                return Err(EngineInputError::RepeatedSignal { index });
            }
        }
        Ok(())
    }

    pub(crate) fn signal(&self, selector: &SignalSelector) -> Option<f64> {
        self.signals
            .iter()
            .find(|signal| signal.selector == *selector)
            .map(|signal| signal.value)
    }
}

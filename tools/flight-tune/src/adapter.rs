use std::error::Error;
use std::fmt;

use pilotage_trial::Digest;

use crate::ArtifactIdentity;

mod lifecycle;
mod terminal;
mod transition;

pub use lifecycle::{
    CandidateReceipt, RunPreparationReceipt, SampleEvent, ScenarioStartReceipt, SimulatorBackend,
    SimulatorVehicleAdapter, SimulatorVehicleFactory, TelemetrySample,
};
pub use terminal::{RunTerminalAdapter, RunTerminalCapabilities};
pub(crate) use transition::planning_context_digest;
pub use transition::{
    CANDIDATE_TRANSITION_RECEIPT_SCHEMA_VERSION, CandidateTransitionReceipt,
    CandidateTransitionReference, CandidateTransitionRequest,
};

/// A typed error from a simulator or vehicle adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterError {
    detail: String,
}

impl AdapterError {
    /// Creates an adapter error with stable diagnostic text.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for AdapterError {}

/// A typed error from a streaming gate or metric evaluator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatorError {
    detail: String,
}

impl EvaluatorError {
    /// Creates an evaluator error with stable diagnostic text.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for EvaluatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for EvaluatorError {}

/// The exact challenge for one validated simulator session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionChallenge {
    session_digest: Digest,
}

impl SessionChallenge {
    pub(crate) const fn new(session_digest: Digest) -> Self {
        Self { session_digest }
    }

    /// Returns the requested tuning session digest.
    #[must_use]
    pub const fn session_digest(&self) -> Digest {
        self.session_digest
    }
}

/// The simulator handshake response for a session challenge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatorSessionReceipt {
    /// The accepted tuning session digest.
    pub session_digest: Digest,
    /// The running simulator identity digest.
    pub simulator_digest: Digest,
    /// The loaded airframe identity digest.
    pub airframe_digest: Digest,
}

/// An opaque capability for one validated simulator session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatorCapability {
    session_digest: Digest,
    simulator_digest: Digest,
    airframe_digest: Digest,
}

impl SimulatorCapability {
    pub(crate) const fn new(receipt: SimulatorSessionReceipt) -> Self {
        Self {
            session_digest: receipt.session_digest,
            simulator_digest: receipt.simulator_digest,
            airframe_digest: receipt.airframe_digest,
        }
    }

    /// Returns the validated tuning session digest.
    #[must_use]
    pub const fn session_digest(&self) -> Digest {
        self.session_digest
    }

    /// Binds a simulator vehicle adapter to this capability.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the binding receipt does not match.
    pub fn bind_vehicle<A>(
        &self,
        adapter: A,
        receipt: VehicleBindingReceipt,
    ) -> Result<VehicleBinding<A>, AdapterError> {
        if receipt.session_digest != self.session_digest {
            return Err(AdapterError::new(
                "vehicle binding did not accept the simulator session",
            ));
        }
        Ok(VehicleBinding {
            adapter,
            receipt,
            transition: None,
        })
    }

    /// Binds a vehicle adapter and its transition policy to this capability.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when a binding does not match this session or
    /// has an invalid transition identity.
    pub fn bind_vehicle_with_transition<A>(
        &self,
        adapter: A,
        receipt: VehicleBindingReceipt,
        transition: TransitionBindingReceipt,
    ) -> Result<VehicleBinding<A>, AdapterError> {
        if receipt.session_digest != self.session_digest
            || transition.session_digest() != self.session_digest
        {
            return Err(AdapterError::new(
                "vehicle binding did not accept the simulator session",
            ));
        }
        transition.validate()?;
        Ok(VehicleBinding {
            adapter,
            receipt,
            transition: Some(transition),
        })
    }
}

/// The receipt that binds a vehicle adapter to a simulator session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleBindingReceipt {
    /// The accepted simulator session digest.
    pub session_digest: Digest,
    /// The bound vehicle implementation digest.
    pub vehicle_digest: Digest,
}

/// The transition policy that is fixed to one simulator session binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionBindingReceipt {
    session_digest: Digest,
    validator: ArtifactIdentity,
    adjacency_policy_digest: Digest,
}

impl TransitionBindingReceipt {
    /// Creates a transition-policy binding for one simulator session.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when an identity is invalid.
    pub fn new(
        session_digest: Digest,
        validator: ArtifactIdentity,
        adjacency_policy_digest: Digest,
    ) -> Result<Self, AdapterError> {
        let receipt = Self {
            session_digest,
            validator,
            adjacency_policy_digest,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    /// Returns the bound simulator session identity.
    #[must_use]
    pub const fn session_digest(&self) -> Digest {
        self.session_digest
    }

    /// Returns the bound transition validator identity.
    #[must_use]
    pub const fn validator(&self) -> &ArtifactIdentity {
        &self.validator
    }

    /// Returns the bound adjacency-policy identity.
    #[must_use]
    pub const fn adjacency_policy_digest(&self) -> Digest {
        self.adjacency_policy_digest
    }

    fn validate(&self) -> Result<(), AdapterError> {
        if self.session_digest.is_zero()
            || self.adjacency_policy_digest.is_zero()
            || self.validator.validate().is_err()
        {
            return Err(AdapterError::new(
                "candidate-transition binding has an invalid identity",
            ));
        }
        Ok(())
    }

    fn validate_request(&self, request: &CandidateTransitionRequest) -> Result<(), AdapterError> {
        if request.session_digest() != self.session_digest
            || request.validator() != &self.validator
            || request.adjacency_policy_digest() != self.adjacency_policy_digest
        {
            return Err(AdapterError::new(
                "candidate-transition request differs from the vehicle binding",
            ));
        }
        Ok(())
    }
}

/// A vehicle adapter that has a validated simulator session binding.
pub struct VehicleBinding<A> {
    adapter: A,
    receipt: VehicleBindingReceipt,
    transition: Option<TransitionBindingReceipt>,
}

impl<A> VehicleBinding<A> {
    pub(crate) const fn receipt(&self) -> VehicleBindingReceipt {
        self.receipt
    }

    pub(crate) const fn transition_receipt(&self) -> Option<&TransitionBindingReceipt> {
        self.transition.as_ref()
    }

    pub(crate) fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }
}

impl<A: SimulatorVehicleAdapter> VehicleBinding<A> {
    /// Authorizes one exact transition without controller mutation.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the session or policy binding differs,
    /// or when the adapter rejects the transition.
    pub(crate) fn authorize_candidate_transition(
        &self,
        request: &CandidateTransitionRequest,
    ) -> Result<CandidateTransitionReceipt, AdapterError> {
        let transition = self.transition.as_ref().ok_or_else(|| {
            AdapterError::new("vehicle binding has no candidate-transition policy")
        })?;
        transition.validate_request(request)?;
        let receipt = self.adapter.authorize_candidate_transition(request)?;
        receipt
            .validate_for(request)
            .map_err(|error| AdapterError::new(error.to_string()))?;
        Ok(receipt)
    }
}

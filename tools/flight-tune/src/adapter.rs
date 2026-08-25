use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::time::Duration;

use pilotage_trial::Digest;

use crate::{ArtifactIdentity, Candidate, RunExecutionContext, ScenarioRef};

mod transition;

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

/// The receipt for an applied candidate and its controller readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateReceipt {
    /// The tuning session that received the candidate.
    pub session_digest: Digest,
    /// The requested candidate digest.
    pub requested_digest: Digest,
    /// The digest reported by the apply operation.
    pub applied_digest: Digest,
    /// The digest reconstructed from controller readback.
    pub readback_digest: Digest,
    /// The exact run intent, or no intent for idle reconciliation.
    pub run_intent_digest: Option<Digest>,
}

/// The receipt for a prepared simulator run intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunPreparationReceipt {
    /// The tuning session that owns the run.
    pub session_digest: Digest,
    /// The exact prepared run intent digest.
    pub run_intent_digest: Digest,
}

/// The receipt for the scenario that started in the simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenarioStartReceipt {
    /// The tuning session that owns the run.
    pub session_digest: Digest,
    /// The digest of the applied scenario artifact.
    pub applied_scenario_digest: Digest,
    /// The applied deterministic run seed.
    pub seed: u64,
    /// The exact started run intent digest.
    pub run_intent_digest: Digest,
}

/// One ordered simulator telemetry sample.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetrySample {
    /// The zero-based sample sequence in this run.
    pub sequence: u64,
    /// The elapsed simulator time in milliseconds.
    pub elapsed_ms: u64,
    /// Named telemetry values for streaming gate and metric evaluation.
    pub values: BTreeMap<String, f64>,
}

/// The result of one bounded sample request.
#[derive(Debug, Clone, PartialEq)]
pub enum SampleEvent {
    /// The backend supplied one telemetry sample.
    Sample(TelemetrySample),
    /// The scenario completed normally.
    Complete,
    /// The backend did not supply a sample before the requested timeout.
    TimedOut,
}

/// A simulator backend with an explicit run lifecycle.
pub trait SimulatorBackend {
    /// Returns the exact simulator implementation identity.
    fn simulator_identity(&self) -> &ArtifactIdentity;

    /// Returns the exact loaded airframe identity.
    fn airframe_identity(&self) -> &ArtifactIdentity;

    /// Opens and validates one simulator session.
    fn open_session_blocking(
        &mut self,
        challenge: &SessionChallenge,
    ) -> Result<SimulatorSessionReceipt, AdapterError>;

    /// Prepares one exact durable run intent.
    fn prepare_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
        scenario: &ScenarioRef,
    ) -> Result<RunPreparationReceipt, AdapterError>;

    /// Starts the prepared scenario and returns the applied artifact receipt.
    fn start_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
    ) -> Result<ScenarioStartReceipt, AdapterError>;

    /// Requests the next telemetry sample with a finite timeout.
    fn sample_blocking(&mut self, timeout: Duration) -> Result<SampleEvent, AdapterError>;

    /// Stops the active scenario.
    fn stop_blocking(&mut self) -> Result<(), AdapterError>;

    /// Restores the simulator to a clean idle state.
    fn cleanup_blocking(&mut self) -> Result<(), AdapterError>;
}

/// A vehicle adapter that can activate a candidate only with a simulator binding.
pub trait SimulatorVehicleAdapter {
    /// Validates one exact candidate transition without external mutation.
    ///
    /// # Errors
    ///
    /// Returns an error when the source, target, or adjacency policy rejects
    /// the transition. The default rejects adapters that do not implement the
    /// transition contract.
    fn authorize_candidate_transition(
        &self,
        _request: &CandidateTransitionRequest,
    ) -> Result<CandidateTransitionReceipt, AdapterError> {
        Err(AdapterError::new(
            "vehicle adapter has no candidate-transition validator",
        ))
    }

    /// Ensures that the settled candidate is active during reconciliation.
    ///
    /// The operation must not write controller state when the requested
    /// candidate is already active. This rule makes restart reconciliation
    /// safe to repeat.
    fn ensure_settled_candidate_blocking(
        &mut self,
        capability: &SimulatorCapability,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError>;

    /// Ensures that a candidate is active for one exact durable run intent.
    ///
    /// The receipt must include the digest of `context`. The operation must
    /// not write controller state when the requested candidate is active.
    fn ensure_candidate_for_run_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _context: &RunExecutionContext,
        _candidate: &Candidate,
        _candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        Err(AdapterError::new(
            "vehicle adapter has no run-intent candidate activation",
        ))
    }
}

/// A factory that binds a vehicle adapter to a validated simulator session.
pub trait SimulatorVehicleFactory {
    /// The bound adapter type.
    type Adapter: SimulatorVehicleAdapter;

    /// Returns the exact vehicle implementation identity.
    fn vehicle_identity(&self) -> &ArtifactIdentity;

    /// Returns the exact candidate-transition validator identity.
    fn transition_validator_identity(&self) -> &ArtifactIdentity;

    /// Returns the exact vehicle adjacency-policy identity.
    fn adjacency_policy_digest(&self) -> Digest;

    /// Creates a vehicle binding for the validated simulator session.
    fn bind_blocking(
        self,
        capability: &SimulatorCapability,
    ) -> Result<VehicleBinding<Self::Adapter>, AdapterError>;
}

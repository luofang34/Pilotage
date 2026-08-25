use std::collections::BTreeMap;
use std::time::Duration;

use pilotage_trial::Digest;

use super::{
    AdapterError, CandidateTransitionReceipt, CandidateTransitionRequest, SessionChallenge,
    SimulatorCapability, SimulatorSessionReceipt, VehicleBinding,
};
use crate::{ArtifactIdentity, Candidate, RunExecutionContext, ScenarioRef};

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

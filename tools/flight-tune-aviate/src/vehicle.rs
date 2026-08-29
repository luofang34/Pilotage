//! The production Aviate vehicle binding.
//!
//! A generic candidate is a map of numbers. This module is where it stops
//! being generic: each candidate maps to one exact Aviate feel profile,
//! the complete mapped profile is validated, and the transition from the
//! current incumbent to that exact target is authorized. All three happen
//! before the process, the simulator, or the vehicle is touched, so a
//! candidate that cannot fly is refused while it is still a value.
//!
//! The factory declares the identities the runtime binds: the vehicle
//! implementation, its action port, its transition validator, and its
//! adjacency policy.
//!
//! SIM / NOT FOR FLIGHT.

use flight_tune::{
    AdapterError, ArtifactIdentity, Candidate, CandidateReceipt, CandidateTransitionReceipt,
    CandidateTransitionRequest, Digest, RunExecutionContext, SimulatorCapability,
    SimulatorVehicleAdapter, SimulatorVehicleFactory, TransitionBindingReceipt, TuneError,
    VehicleBinding, VehicleBindingReceipt, scenario_runtime_identity,
};
use pilotage_control_feel::{FeelDigest, ValidatedFlightFeelProfile};
use sha2::{Digest as _, Sha256};

use crate::SupervisedProcessRequest;
use crate::action_port::aviate_action_port_identity;
use crate::transition_authorization::TransitionValidator;

/// The stable name of the Aviate vehicle implementation identity.
pub const VEHICLE_ID: &str = "pilotage-aviate-vehicle-v1";

const VEHICLE_IDENTITY_DOMAIN: &[u8] = b"pilotage-aviate-vehicle-v1\0";

/// Maps one generic candidate to one exact Aviate feel profile.
///
/// The mapping is the vehicle's own: it names which parameters it reads
/// and how they become a complete profile. A candidate that does not
/// state a parameter the mapping needs is refused, never defaulted.
pub trait CandidateFeelMapping {
    /// The exact identity of this mapping and its configuration.
    fn mapping_identity(&self) -> &ArtifactIdentity;

    /// Maps one candidate to one complete, validated feel profile.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the candidate omits a parameter the
    /// mapping needs, or when the complete mapped profile is not valid.
    fn map(&self, candidate: &Candidate) -> Result<ValidatedFlightFeelProfile, AdapterError>;
}

/// Writes and reads back the exact control law of one Aviate vehicle.
pub trait AviateFeelController {
    /// Reads back the complete profile the controller currently holds.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the controller cannot be read or
    /// reports a profile that is not valid.
    fn readback_blocking(&mut self) -> Result<ValidatedFlightFeelProfile, AdapterError>;

    /// Writes one complete profile and returns what the controller took.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the controller refuses the profile.
    fn apply_blocking(&mut self, profile: &ValidatedFlightFeelProfile) -> Result<(), AdapterError>;
}

/// The production Aviate vehicle adapter.
#[derive(Debug)]
pub struct AviateVehicleAdapter<M, C> {
    mapping: M,
    controller: C,
    validator: TransitionValidator,
    session_digest: Digest,
    settled: Option<Digest>,
}

impl<M: CandidateFeelMapping, C: AviateFeelController> AviateVehicleAdapter<M, C> {
    /// Creates one adapter bound to a validated simulator session.
    ///
    /// The capability supplies the session the adapter answers for, so an
    /// adapter cannot be built for a session that was never validated.
    #[must_use]
    pub fn bind(
        mapping: M,
        controller: C,
        validator: TransitionValidator,
        capability: &SimulatorCapability,
    ) -> Self {
        Self {
            mapping,
            controller,
            validator,
            session_digest: capability.session_digest(),
            settled: None,
        }
    }

    /// The candidate this adapter last settled on the controller.
    #[must_use]
    pub const fn settled_candidate_digest(&self) -> Option<Digest> {
        self.settled
    }

    /// The transition validator this adapter authorizes through.
    #[must_use]
    pub const fn validator(&self) -> &TransitionValidator {
        &self.validator
    }

    /// Activates one candidate, writing nothing when it is already active.
    ///
    /// The controller is read back first. A readback that already carries
    /// the exact mapped profile returns a receipt with no write at all,
    /// which is what makes restart reconciliation safe to repeat.
    fn ensure_blocking(
        &mut self,
        capability: &SimulatorCapability,
        candidate: &Candidate,
        candidate_digest: Digest,
        run_intent_digest: Option<Digest>,
    ) -> Result<CandidateReceipt, AdapterError> {
        if capability.session_digest() != self.session_digest {
            return Err(AdapterError::new(
                "the capability does not name the bound tuning session",
            ));
        }
        if candidate_digest.is_zero() {
            return Err(AdapterError::new("the requested candidate has no identity"));
        }
        let target = self.mapping.map(candidate)?;
        let target_feel = feel_digest(&target)?;
        if feel_digest(&self.controller.readback_blocking()?)? != target_feel {
            self.controller.apply_blocking(&target)?;
        }
        if feel_digest(&self.controller.readback_blocking()?)? != target_feel {
            return Err(AdapterError::new(
                "the controller readback is not the exact mapped feel profile",
            ));
        }
        self.settled = Some(candidate_digest);
        Ok(CandidateReceipt {
            session_digest: self.session_digest,
            requested_digest: candidate_digest,
            applied_digest: candidate_digest,
            readback_digest: candidate_digest,
            run_intent_digest,
        })
    }
}

impl<M: CandidateFeelMapping, C: AviateFeelController> SimulatorVehicleAdapter
    for AviateVehicleAdapter<M, C>
{
    fn authorize_candidate_transition(
        &self,
        request: &CandidateTransitionRequest,
    ) -> Result<CandidateTransitionReceipt, AdapterError> {
        if request.session_digest() != self.session_digest {
            return Err(AdapterError::new(
                "the transition request does not name the bound tuning session",
            ));
        }
        // A later transition is checked against the candidate that is
        // actually active, not against the one the search started from.
        if let Some(settled) = self.settled
            && request.source_candidate_digest() != settled
        {
            return Err(AdapterError::new(
                "the transition source is not the current incumbent",
            ));
        }
        // The complete mapped target has to be a profile the vehicle would
        // load, and it is checked before any controller is written to.
        let _target = self.mapping.map(request.target())?;
        self.validator.authorize(request)
    }

    fn ensure_settled_candidate_blocking(
        &mut self,
        capability: &SimulatorCapability,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        self.ensure_blocking(capability, candidate, candidate_digest, None)
    }

    fn ensure_candidate_for_run_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        let run_intent_digest = context
            .digest()
            .map_err(|source| AdapterError::new(source.to_string()))?;
        if context.candidate_digest() != candidate_digest {
            return Err(AdapterError::new("the run intent names another candidate"));
        }
        self.ensure_blocking(
            capability,
            candidate,
            candidate_digest,
            Some(run_intent_digest),
        )
    }
}

/// The production Aviate simulator vehicle factory.
#[derive(Debug)]
pub struct AviateVehicleFactory<M, C> {
    mapping: M,
    controller: C,
    validator: TransitionValidator,
    vehicle: ArtifactIdentity,
    action_port: ArtifactIdentity,
    runtime_identity: ArtifactIdentity,
}

impl<M: CandidateFeelMapping, C: AviateFeelController> AviateVehicleFactory<M, C> {
    /// Creates one factory for an exact mapping, controller, and rule.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when a bound identity is not valid.
    pub fn new(
        mapping: M,
        controller: C,
        validator: TransitionValidator,
        runtime_identity: ArtifactIdentity,
    ) -> Result<Self, AdapterError> {
        let vehicle = vehicle_identity(
            mapping.mapping_identity(),
            validator.identity(),
            validator.policy_digest(),
            &runtime_identity,
        )
        .map_err(|source| AdapterError::new(source.to_string()))?;
        let action_port = aviate_action_port_identity(&runtime_identity)
            .map_err(|source| AdapterError::new(source.to_string()))?;
        Ok(Self {
            mapping,
            controller,
            validator,
            vehicle,
            action_port,
            runtime_identity,
        })
    }

    /// The runtime implementation identity this factory binds.
    #[must_use]
    pub const fn runtime_identity(&self) -> &ArtifactIdentity {
        &self.runtime_identity
    }

    /// The final engine and vehicle action-port identity.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the action-port identity is not valid.
    pub fn scenario_runtime_digest(&self) -> Result<Digest, AdapterError> {
        scenario_runtime_identity(&self.action_port)
            .map(|identity| identity.digest)
            .map_err(|source| AdapterError::new(source.to_string()))
    }
}

impl<M: CandidateFeelMapping, C: AviateFeelController> SimulatorVehicleFactory
    for AviateVehicleFactory<M, C>
{
    type Adapter = AviateVehicleAdapter<M, C>;

    fn vehicle_identity(&self) -> &ArtifactIdentity {
        &self.vehicle
    }

    fn scenario_action_port_identity(&self) -> &ArtifactIdentity {
        &self.action_port
    }

    fn transition_validator_identity(&self) -> &ArtifactIdentity {
        self.validator.identity()
    }

    fn adjacency_policy_digest(&self) -> Digest {
        self.validator.policy_digest()
    }

    fn bind_blocking(
        self,
        capability: &SimulatorCapability,
    ) -> Result<VehicleBinding<Self::Adapter>, AdapterError> {
        let session_digest = capability.session_digest();
        let receipt = VehicleBindingReceipt {
            session_digest,
            vehicle_digest: self.vehicle.digest,
            scenario_runtime_digest: self.scenario_runtime_digest()?,
        };
        let transition = TransitionBindingReceipt::new(
            session_digest,
            self.validator.identity().clone(),
            self.validator.policy_digest(),
        )?;
        let adapter =
            AviateVehicleAdapter::bind(self.mapping, self.controller, self.validator, capability);
        capability.bind_vehicle_with_transition(adapter, receipt, transition)
    }
}

/// Binds one exact run intent to a supervised process launch request.
///
/// The supervisor stores the digest with its attestation, so the process
/// that flies a run and the run intent that authorized it are one fact
/// rather than two that happen to line up.
#[must_use]
pub fn bind_run_intent(
    mut request: SupervisedProcessRequest,
    run_intent_digest: Digest,
) -> SupervisedProcessRequest {
    request.run_intent_digest = run_intent_digest;
    request
}

/// Rejects a launch request that does not carry one exact run intent.
///
/// # Errors
///
/// Returns [`AdapterError`] when the request names another run intent.
pub fn require_run_intent(
    request: &SupervisedProcessRequest,
    run_intent_digest: Digest,
) -> Result<(), AdapterError> {
    if request.run_intent_digest == run_intent_digest && !run_intent_digest.is_zero() {
        return Ok(());
    }
    Err(AdapterError::new(
        "the supervised process request does not carry the exact run intent",
    ))
}

/// The exact identity of one Aviate vehicle implementation.
///
/// # Errors
///
/// Returns [`TuneError`] when the identity cannot be named.
pub fn vehicle_identity(
    mapping: &ArtifactIdentity,
    validator: &ArtifactIdentity,
    adjacency_policy_digest: Digest,
    runtime: &ArtifactIdentity,
) -> Result<ArtifactIdentity, TuneError> {
    let source = include_bytes!("vehicle.rs");
    let mut hasher = Sha256::new();
    hasher.update(VEHICLE_IDENTITY_DOMAIN);
    hasher.update((source.len() as u64).to_le_bytes());
    hasher.update(source);
    for identity in [mapping, validator, runtime] {
        // `ArtifactIdentity::new` is the harness's public validation path:
        // it refuses an empty name and a zero digest.
        ArtifactIdentity::new(identity.id.clone(), identity.digest)?;
        hasher.update((identity.id.len() as u64).to_le_bytes());
        hasher.update(identity.id.as_bytes());
        hasher.update(identity.digest.as_bytes());
    }
    hasher.update(adjacency_policy_digest.as_bytes());
    ArtifactIdentity::new(VEHICLE_ID, Digest::from_bytes(hasher.finalize().into()))
}

fn feel_digest(profile: &ValidatedFlightFeelProfile) -> Result<Digest, AdapterError> {
    FeelDigest::calculate(profile)
        .map(|digest| Digest::from_bytes(*digest.as_bytes()))
        .map_err(|source| AdapterError::new(source.to_string()))
}

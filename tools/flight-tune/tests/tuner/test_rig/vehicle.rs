use flight_tune::{
    AdapterError, ArtifactIdentity, Candidate, CandidateReceipt, CandidateTransitionReceipt,
    CandidateTransitionRequest, Digest, RunExecutionContext, SimulatorCapability,
    SimulatorVehicleAdapter, SimulatorVehicleFactory, TransitionBindingReceipt, VehicleBinding,
    VehicleBindingAcquisition, VehicleBindingReceipt, VehicleBindingRollback,
};

use super::{FakeHandle, identity};

pub struct FakeVehicle {
    pub(super) state: FakeHandle,
}

impl SimulatorVehicleAdapter for FakeVehicle {
    fn authorize_candidate_transition(
        &self,
        request: &CandidateTransitionRequest,
    ) -> Result<CandidateTransitionReceipt, AdapterError> {
        let source = candidate_gain(request.source())?;
        let target = candidate_gain(request.target())?;
        let mut state = self.state.0.borrow_mut();
        state.transition.authorization_count = state.transition.authorization_count.wrapping_add(1);
        state.lifecycle.push("authorize_transition".to_owned());
        state.transition.checks.push((source, target));
        if state
            .transition
            .maximum_delta
            .is_some_and(|limit| (target - source).abs() > limit)
        {
            return Err(AdapterError::new(
                "the candidate transition exceeds the vehicle adjacency limit",
            ));
        }
        drop(state);
        CandidateTransitionReceipt::authorized(request)
            .map_err(|error| AdapterError::new(error.to_string()))
    }

    fn ensure_settled_candidate_blocking(
        &mut self,
        capability: &SimulatorCapability,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        activate_candidate(&self.state, capability, candidate, candidate_digest, None)
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
            .map_err(|error| AdapterError::new(error.to_string()))?;
        let mut state = self.state.0.borrow_mut();
        state.transition.vehicle_contexts.push(context.clone());
        let receipt_intent = if state.transition.bad_vehicle_intent {
            Digest::from_bytes([95; 32])
        } else {
            run_intent_digest
        };
        drop(state);
        activate_candidate(
            &self.state,
            capability,
            candidate,
            candidate_digest,
            Some(receipt_intent),
        )
    }
}

fn activate_candidate(
    handle: &FakeHandle,
    capability: &SimulatorCapability,
    candidate: &Candidate,
    candidate_digest: Digest,
    run_intent_digest: Option<Digest>,
) -> Result<CandidateReceipt, AdapterError> {
    let gain = candidate
        .parameters()
        .get("gain")
        .copied()
        .ok_or_else(|| AdapterError::new("candidate has no gain"))?;
    let mut state = handle.0.borrow_mut();
    state.vehicle.ensure_count = state.vehicle.ensure_count.wrapping_add(1);
    let wrote_candidate = state.vehicle.active_candidate_digest != Some(candidate_digest);
    if wrote_candidate {
        state.vehicle.gain = gain;
        state.vehicle.active_candidate_digest = Some(candidate_digest);
        state.vehicle.apply_count = state.vehicle.apply_count.wrapping_add(1);
        state.lifecycle.push("apply".to_owned());
    }
    let bad_readback = state.vehicle.bad_candidate_readback_on_ensure
        == Some(state.vehicle.ensure_count)
        || (wrote_candidate
            && state.vehicle.bad_candidate_readback_on_apply == Some(state.vehicle.apply_count));
    let readback = if bad_readback {
        Digest::from_bytes([99; 32])
    } else {
        candidate_digest
    };
    Ok(CandidateReceipt {
        session_digest: capability.session_digest(),
        requested_digest: candidate_digest,
        applied_digest: candidate_digest,
        readback_digest: readback,
        run_intent_digest,
    })
}

pub struct FakeFactory {
    state: FakeHandle,
    identity: ArtifactIdentity,
    action_port_identity: ArtifactIdentity,
    transition_validator: ArtifactIdentity,
    adjacency_policy_digest: Digest,
    allow_binding: bool,
}

impl FakeFactory {
    pub fn new(state: FakeHandle) -> Self {
        let vehicle_identity = identity("vehicle", "fake-controller-v1");
        Self {
            state,
            action_port_identity: identity("vehicle", "fake-controller-v1"),
            identity: vehicle_identity,
            transition_validator: identity("transition-validator", "fake-validator-v1"),
            adjacency_policy_digest: digest_text("fake-adjacency-policy-v1"),
            allow_binding: true,
        }
    }

    pub fn hardware_like(state: FakeHandle) -> Self {
        let vehicle_identity = identity("vehicle", "hardware-like-controller");
        Self {
            state,
            action_port_identity: identity("vehicle", "fake-controller-v1"),
            identity: vehicle_identity,
            transition_validator: identity("transition-validator", "hardware-validator-v1"),
            adjacency_policy_digest: digest_text("hardware-adjacency-policy-v1"),
            allow_binding: false,
        }
    }

    pub fn with_transition_validator(state: FakeHandle, content: &str) -> Self {
        let mut factory = Self::new(state);
        factory.transition_validator = identity("transition-validator", content);
        factory
    }

    pub fn with_action_port_identity(state: FakeHandle, content: &str) -> Self {
        let mut factory = Self::new(state);
        factory.action_port_identity = identity("scenario-action-port", content);
        factory
    }

    pub fn with_adjacency_policy(state: FakeHandle, content: &str) -> Self {
        let mut factory = Self::new(state);
        factory.adjacency_policy_digest = digest_text(content);
        factory
    }

    /// Names the binding acquisition this factory answers for.
    pub fn binding_acquisition(&self, session_digest: Digest) -> VehicleBindingAcquisition {
        VehicleBindingAcquisition::new(
            session_digest,
            self.identity.digest,
            runtime_digest(&self.action_port_identity),
        )
    }
}

/// Releases or contains one exact fake vehicle binding.
pub struct FakeVehicleRollback {
    state: FakeHandle,
    vehicle_digest: Digest,
    scenario_runtime_digest: Digest,
}

impl VehicleBindingRollback for FakeVehicleRollback {
    fn release_binding_blocking(
        &mut self,
        acquisition: &VehicleBindingAcquisition,
    ) -> Result<(), AdapterError> {
        if acquisition.session_digest().is_zero()
            || acquisition.vehicle_digest() != self.vehicle_digest
            || acquisition.scenario_runtime_digest() != self.scenario_runtime_digest
        {
            return Err(AdapterError::new(
                "the acquisition names another vehicle binding",
            ));
        }
        let mut state = self.state.0.borrow_mut();
        state.open_order.push("release_binding".to_owned());
        state.vehicle.release_count = state.vehicle.release_count.wrapping_add(1);
        if state.vehicle.fail_release_on == Some(state.vehicle.release_count) {
            return Err(AdapterError::new(
                "the vehicle did not acknowledge the binding release",
            ));
        }
        state.vehicle.bound = false;
        Ok(())
    }
}

impl SimulatorVehicleFactory for FakeFactory {
    type Adapter = FakeVehicle;
    type Rollback = FakeVehicleRollback;

    fn vehicle_identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn rollback_handle(&self) -> Self::Rollback {
        FakeVehicleRollback {
            state: self.state.clone(),
            vehicle_digest: self.identity.digest,
            scenario_runtime_digest: runtime_digest(&self.action_port_identity),
        }
    }

    fn scenario_action_port_identity(&self) -> &ArtifactIdentity {
        &self.action_port_identity
    }

    fn transition_validator_identity(&self) -> &ArtifactIdentity {
        &self.transition_validator
    }

    fn adjacency_policy_digest(&self) -> Digest {
        self.adjacency_policy_digest
    }

    fn bind_blocking(
        self,
        capability: &SimulatorCapability,
    ) -> Result<VehicleBinding<Self::Adapter>, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.vehicle.bind_count = state.vehicle.bind_count.wrapping_add(1);
        state.open_order.push("bind".to_owned());
        // The binding exists from here on, whether or not the rest of the
        // bind completes, so a failure below is a partial bind.
        state.vehicle.bound = true;
        let fail_bind = state.vehicle.fail_bind;
        let bad_receipt = state.vehicle.bad_binding_receipt;
        drop(state);
        if fail_bind {
            return Err(AdapterError::new(
                "the vehicle stopped part way through the binding",
            ));
        }
        if !self.allow_binding {
            return Err(AdapterError::new(
                "hardware-like adapter has no simulator session binding",
            ));
        }
        let transition = TransitionBindingReceipt::new(
            capability.session_digest(),
            self.transition_validator,
            self.adjacency_policy_digest,
        )?;
        capability.bind_vehicle_with_transition(
            FakeVehicle { state: self.state },
            VehicleBindingReceipt {
                session_digest: capability.session_digest(),
                vehicle_digest: self.identity.digest,
                scenario_runtime_digest: if bad_receipt {
                    Digest::from_bytes([61; 32])
                } else {
                    runtime_digest(&self.action_port_identity)
                },
            },
            transition,
        )
    }
}

fn runtime_digest(action_port: &ArtifactIdentity) -> Digest {
    flight_tune::scenario_runtime_identity(action_port)
        .expect("scenario runtime identity")
        .digest
}

fn candidate_gain(candidate: &Candidate) -> Result<f64, AdapterError> {
    candidate
        .parameters()
        .get("gain")
        .copied()
        .ok_or_else(|| AdapterError::new("candidate has no gain"))
}

fn digest_text(content: &str) -> Digest {
    identity("digest", content).digest
}

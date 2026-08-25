use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use flight_tune::{
    AdapterError, ArtifactIdentity, Candidate, CandidateReceipt, CandidateTransitionReceipt,
    CandidateTransitionRequest, Digest, RunExecutionContext, RunPreparationReceipt, SampleEvent,
    ScenarioRef, ScenarioStartReceipt, SessionChallenge, SimulatorBackend, SimulatorCapability,
    SimulatorSessionReceipt, SimulatorVehicleAdapter, SimulatorVehicleFactory, TelemetrySample,
    TransitionBindingReceipt, VehicleBinding, VehicleBindingReceipt,
};

#[path = "test_rig/cleanup_fault.rs"]
mod cleanup_fault;
#[path = "test_rig/scoring.rs"]
mod scoring;
#[path = "test_rig/vehicle_state.rs"]
mod vehicle_state;

pub use cleanup_fault::FakeCleanupFault;
#[allow(unused_imports)]
pub use scoring::{
    EnvelopeGates, ObservedViews, QuadraticMetric, SequenceStrategy, assert_receipt_error,
    candidate, stage,
};
pub use vehicle_state::FakeVehicleState;

#[derive(Debug, Default)]
pub struct FakeState {
    pub vehicle: FakeVehicleState,
    pub open_session_count: usize,
    pub prepare_count: usize,
    pub start_count: usize,
    pub sample_count: usize,
    pub sample_poll_count: usize,
    pub stop_count: usize,
    pub cleanup_count: usize,
    pub metric_observe_count: usize,
    pub gate_begin_count: usize,
    pub gate_evaluate_count: usize,
    pub gate_finish_count: usize,
    pub gate_cancel_count: usize,
    pub metric_begin_count: usize,
    pub metric_finish_count: usize,
    pub metric_cancel_count: usize,
    pub scenario_runs: Vec<(String, u64, f64)>,
    pub transition: FakeTransitionState,
    pub lifecycle: Vec<String>,
    pub current_scenario: Option<ScenarioRef>,
    pub current_seed: u64,
    pub next_sequence: u64,
    pub panic_on_prepare: Option<usize>,
    pub panic_on_start: Option<usize>,
    pub cleanup_fault: FakeCleanupFault,
    pub change_head_on_prepare: Option<PathBuf>,
    pub bad_scenario_readback: bool,
    pub timeout_next_sample: bool,
    pub complete_without_sample: bool,
}

#[derive(Debug, Default)]
pub struct FakeTransitionState {
    pub authorization_count: usize,
    pub checks: Vec<(f64, f64)>,
    pub maximum_delta: Option<f64>,
    pub prepared_contexts: Vec<RunExecutionContext>,
    pub started_contexts: Vec<RunExecutionContext>,
    pub vehicle_contexts: Vec<RunExecutionContext>,
    pub bad_preparation_intent: bool,
    pub bad_start_intent: bool,
    pub bad_vehicle_intent: bool,
}

#[derive(Clone)]
pub struct FakeHandle(pub Rc<RefCell<FakeState>>);

impl FakeHandle {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(FakeState::default())))
    }
}

pub struct FakeBackend {
    state: FakeHandle,
    simulator: ArtifactIdentity,
    airframe: ArtifactIdentity,
}

impl FakeBackend {
    pub fn new(state: FakeHandle) -> Self {
        Self::with_simulator_id(state, "fake-simulator-v1")
    }

    pub fn with_simulator_id(state: FakeHandle, id: &str) -> Self {
        Self {
            state,
            simulator: identity("simulator", id),
            airframe: identity("airframe", "default-airframe"),
        }
    }
}

impl SimulatorBackend for FakeBackend {
    fn simulator_identity(&self) -> &ArtifactIdentity {
        &self.simulator
    }

    fn airframe_identity(&self) -> &ArtifactIdentity {
        &self.airframe
    }

    fn open_session_blocking(
        &mut self,
        challenge: &SessionChallenge,
    ) -> Result<SimulatorSessionReceipt, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.open_session_count = state.open_session_count.wrapping_add(1);
        state.lifecycle.push("open_session".to_owned());
        drop(state);
        Ok(SimulatorSessionReceipt {
            session_digest: challenge.session_digest(),
            simulator_digest: self.simulator.digest,
            airframe_digest: self.airframe.digest,
        })
    }

    fn prepare_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
        scenario: &ScenarioRef,
    ) -> Result<RunPreparationReceipt, AdapterError> {
        let run_intent_digest = context
            .digest()
            .map_err(|error| AdapterError::new(error.to_string()))?;
        let mut state = self.state.0.borrow_mut();
        state.prepare_count = state.prepare_count.wrapping_add(1);
        state.lifecycle.push("prepare".to_owned());
        state.transition.prepared_contexts.push(context.clone());
        if state.panic_on_prepare == Some(state.prepare_count) {
            panic!("simulated process stop after AttemptPrepared");
        }
        let head_change = state.change_head_on_prepare.take();
        state.current_scenario = Some(scenario.clone());
        state.current_seed = context.seed();
        state.next_sequence = 0;
        let receipt_intent = if state.transition.bad_preparation_intent {
            Digest::from_bytes([97; 32])
        } else {
            run_intent_digest
        };
        drop(state);
        if let Some(root) = head_change {
            change_head_digest(&root);
        }
        Ok(RunPreparationReceipt {
            session_digest: capability.session_digest(),
            run_intent_digest: receipt_intent,
        })
    }

    fn start_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
    ) -> Result<ScenarioStartReceipt, AdapterError> {
        let run_intent_digest = context
            .digest()
            .map_err(|error| AdapterError::new(error.to_string()))?;
        let mut state = self.state.0.borrow_mut();
        state.start_count = state.start_count.wrapping_add(1);
        state.lifecycle.push("start".to_owned());
        if state.transition.prepared_contexts.last() != Some(context) {
            return Err(AdapterError::new(
                "started run intent differs from prepared run intent",
            ));
        }
        state.transition.started_contexts.push(context.clone());
        if state.panic_on_start == Some(state.start_count) {
            panic!("simulated process stop after candidate activation");
        }
        let scenario = state
            .current_scenario
            .clone()
            .ok_or_else(|| AdapterError::new("scenario was not prepared"))?;
        let seed = state.current_seed;
        let gain = state.vehicle.gain;
        state.scenario_runs.push((scenario.id.clone(), seed, gain));
        let applied_scenario_digest = if state.bad_scenario_readback {
            Digest::from_bytes([98; 32])
        } else {
            scenario.digest
        };
        let receipt_intent = if state.transition.bad_start_intent {
            Digest::from_bytes([96; 32])
        } else {
            run_intent_digest
        };
        Ok(ScenarioStartReceipt {
            session_digest: capability.session_digest(),
            applied_scenario_digest,
            seed,
            run_intent_digest: receipt_intent,
        })
    }

    fn sample_blocking(&mut self, _timeout: Duration) -> Result<SampleEvent, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.sample_poll_count = state.sample_poll_count.wrapping_add(1);
        if state.complete_without_sample {
            return Ok(SampleEvent::Complete);
        }
        if state.timeout_next_sample {
            state.timeout_next_sample = false;
            return Ok(SampleEvent::TimedOut);
        }
        if state.next_sequence > 0 {
            return Ok(SampleEvent::Complete);
        }
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.wrapping_add(1);
        state.sample_count = state.sample_count.wrapping_add(1);
        state.lifecycle.push("sample".to_owned());
        Ok(SampleEvent::Sample(TelemetrySample {
            sequence,
            elapsed_ms: 10,
            values: BTreeMap::from([("gain".to_owned(), state.vehicle.gain)]),
        }))
    }

    fn stop_blocking(&mut self) -> Result<(), AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.stop_count = state.stop_count.wrapping_add(1);
        state.lifecycle.push("stop".to_owned());
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.cleanup_count = state.cleanup_count.wrapping_add(1);
        state.lifecycle.push("cleanup".to_owned());
        state.cleanup_fault.finish(state.cleanup_count)
    }
}

fn change_head_digest(root: &Path) {
    let head = root.join("HEAD.json");
    let mut bytes = std::fs::read(&head).expect("read journal head");
    let digest_tail = bytes.len().checked_sub(3).expect("HEAD digest byte");
    bytes[digest_tail] = if bytes[digest_tail] == b'0' {
        b'1'
    } else {
        b'0'
    };
    std::fs::write(head, bytes).expect("change journal head");
}

pub struct FakeVehicle {
    state: FakeHandle,
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
    transition_validator: ArtifactIdentity,
    adjacency_policy_digest: Digest,
    allow_binding: bool,
}

impl FakeFactory {
    pub fn new(state: FakeHandle) -> Self {
        Self {
            state,
            identity: identity("vehicle", "fake-controller-v1"),
            transition_validator: identity("transition-validator", "fake-validator-v1"),
            adjacency_policy_digest: digest_text("fake-adjacency-policy-v1"),
            allow_binding: true,
        }
    }

    pub fn hardware_like(state: FakeHandle) -> Self {
        Self {
            state,
            identity: identity("vehicle", "hardware-like-controller"),
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

    pub fn with_adjacency_policy(state: FakeHandle, content: &str) -> Self {
        let mut factory = Self::new(state);
        factory.adjacency_policy_digest = digest_text(content);
        factory
    }
}

impl SimulatorVehicleFactory for FakeFactory {
    type Adapter = FakeVehicle;

    fn vehicle_identity(&self) -> &ArtifactIdentity {
        &self.identity
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
        drop(state);
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
            },
            transition,
        )
    }
}

fn candidate_gain(candidate: &Candidate) -> Result<f64, AdapterError> {
    candidate
        .parameters()
        .get("gain")
        .copied()
        .ok_or_else(|| AdapterError::new("candidate has no gain"))
}

fn identity(id: &str, content: &str) -> ArtifactIdentity {
    ArtifactIdentity::from_text(id, content).expect("artifact identity")
}
fn digest_text(content: &str) -> Digest {
    identity("digest", content).digest
}
pub struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    pub fn new(label: &str) -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = test_root().join(format!("flight-tune-{label}-{}-{time}", std::process::id()));
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn test_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/private/tmp")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::temp_dir()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

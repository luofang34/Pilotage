use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use flight_tune::{
    AdapterError, ArtifactIdentity, Candidate, CandidateLineage, CandidateReceipt, Digest,
    EvaluatorError, GateEvaluator, GateOutcome, MetricEvaluator, MetricValues, ParameterBounds,
    PromotionPolicy, Proposal, ProposalContext, ProposalError, ProposalStrategy,
    QualificationPolicy, SampleEvent, ScenarioRef, ScenarioStartReceipt, SearchStage,
    SessionChallenge, SimulatorBackend, SimulatorCapability, SimulatorSessionReceipt,
    SimulatorVehicleAdapter, SimulatorVehicleFactory, TelemetrySample, TrainingObservation,
    TuneError, VehicleBinding, VehicleBindingReceipt,
};

#[derive(Debug, Default)]
pub struct FakeState {
    pub gain: f64,
    pub active_candidate_digest: Option<Digest>,
    pub prepare_count: usize,
    pub ensure_count: usize,
    pub apply_count: usize,
    pub start_count: usize,
    pub sample_count: usize,
    pub stop_count: usize,
    pub cleanup_count: usize,
    pub metric_observe_count: usize,
    pub scenario_runs: Vec<(String, u64, f64)>,
    pub lifecycle: Vec<String>,
    pub current_scenario: Option<ScenarioRef>,
    pub current_seed: u64,
    pub next_sequence: u64,
    pub panic_on_prepare: Option<usize>,
    pub panic_on_start: Option<usize>,
    pub bad_candidate_readback_on_ensure: Option<usize>,
    pub bad_candidate_readback_on_apply: Option<usize>,
    pub bad_scenario_readback: bool,
    pub timeout_next_sample: bool,
    pub complete_without_sample: bool,
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
        self.state
            .0
            .borrow_mut()
            .lifecycle
            .push("open_session".to_owned());
        Ok(SimulatorSessionReceipt {
            session_digest: challenge.session_digest(),
            simulator_digest: self.simulator.digest,
            airframe_digest: self.airframe.digest,
        })
    }

    fn prepare_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        scenario: &ScenarioRef,
        seed: u64,
    ) -> Result<(), AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.prepare_count = state.prepare_count.wrapping_add(1);
        state.lifecycle.push("prepare".to_owned());
        if state.panic_on_prepare == Some(state.prepare_count) {
            panic!("simulated process stop after AttemptPrepared");
        }
        state.current_scenario = Some(scenario.clone());
        state.current_seed = seed;
        state.next_sequence = 0;
        Ok(())
    }

    fn start_blocking(
        &mut self,
        capability: &SimulatorCapability,
    ) -> Result<ScenarioStartReceipt, AdapterError> {
        let mut state = self.state.0.borrow_mut();
        state.start_count = state.start_count.wrapping_add(1);
        state.lifecycle.push("start".to_owned());
        if state.panic_on_start == Some(state.start_count) {
            panic!("simulated process stop after candidate activation");
        }
        let scenario = state
            .current_scenario
            .clone()
            .ok_or_else(|| AdapterError::new("scenario was not prepared"))?;
        let seed = state.current_seed;
        let gain = state.gain;
        state.scenario_runs.push((scenario.id.clone(), seed, gain));
        let applied_scenario_digest = if state.bad_scenario_readback {
            Digest::from_bytes([98; 32])
        } else {
            scenario.digest
        };
        Ok(ScenarioStartReceipt {
            session_digest: capability.session_digest(),
            applied_scenario_digest,
            seed,
        })
    }

    fn sample_blocking(&mut self, _timeout: Duration) -> Result<SampleEvent, AdapterError> {
        let mut state = self.state.0.borrow_mut();
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
            values: BTreeMap::from([("gain".to_owned(), state.gain)]),
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
        Ok(())
    }
}

pub struct FakeVehicle {
    state: FakeHandle,
}

impl SimulatorVehicleAdapter for FakeVehicle {
    fn ensure_candidate_blocking(
        &mut self,
        capability: &SimulatorCapability,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        let gain = candidate
            .parameters()
            .get("gain")
            .copied()
            .ok_or_else(|| AdapterError::new("candidate has no gain"))?;
        let mut state = self.state.0.borrow_mut();
        state.ensure_count = state.ensure_count.wrapping_add(1);
        let wrote_candidate = state.active_candidate_digest != Some(candidate_digest);
        if wrote_candidate {
            state.gain = gain;
            state.active_candidate_digest = Some(candidate_digest);
            state.apply_count = state.apply_count.wrapping_add(1);
            state.lifecycle.push("apply".to_owned());
        }
        let bad_readback = state.bad_candidate_readback_on_ensure == Some(state.ensure_count)
            || (wrote_candidate
                && state.bad_candidate_readback_on_apply == Some(state.apply_count));
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
        })
    }
}

pub struct FakeFactory {
    state: FakeHandle,
    identity: ArtifactIdentity,
    allow_binding: bool,
}

impl FakeFactory {
    pub fn new(state: FakeHandle) -> Self {
        Self {
            state,
            identity: identity("vehicle", "fake-controller-v1"),
            allow_binding: true,
        }
    }

    pub fn hardware_like(state: FakeHandle) -> Self {
        Self {
            state,
            identity: identity("vehicle", "hardware-like-controller"),
            allow_binding: false,
        }
    }
}

impl SimulatorVehicleFactory for FakeFactory {
    type Adapter = FakeVehicle;

    fn vehicle_identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn bind_blocking(
        self,
        capability: &SimulatorCapability,
    ) -> Result<VehicleBinding<Self::Adapter>, AdapterError> {
        if !self.allow_binding {
            return Err(AdapterError::new(
                "hardware-like adapter has no simulator session binding",
            ));
        }
        capability.bind_vehicle(
            FakeVehicle { state: self.state },
            VehicleBindingReceipt {
                session_digest: capability.session_digest(),
                vehicle_digest: self.identity.digest,
            },
        )
    }
}

pub struct EnvelopeGates {
    identity: ArtifactIdentity,
    limit: f64,
}

impl EnvelopeGates {
    pub fn new(limit: f64) -> Self {
        Self {
            identity: identity("gates", &format!("envelope-limit={limit}")),
            limit,
        }
    }
}

impl GateEvaluator for EnvelopeGates {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, _scenario: &ScenarioRef) -> Result<(), EvaluatorError> {
        Ok(())
    }

    fn evaluate(&mut self, sample: &TelemetrySample) -> Result<Vec<GateOutcome>, EvaluatorError> {
        let gain = sample
            .values
            .get("gain")
            .copied()
            .ok_or_else(|| EvaluatorError::new("sample has no gain"))?;
        if gain > self.limit {
            Ok(vec![GateOutcome::fail(
                "envelope",
                "gain exceeded the test envelope",
            )])
        } else {
            Ok(vec![GateOutcome::pass("envelope")])
        }
    }

    fn finish(&mut self) -> Result<(), EvaluatorError> {
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        Ok(())
    }
}

pub struct QuadraticMetric {
    identity: ArtifactIdentity,
    state: FakeHandle,
    gain: Option<f64>,
}

impl QuadraticMetric {
    pub fn new(state: FakeHandle) -> Self {
        Self {
            identity: identity("metric", "quadratic-target-one"),
            state,
            gain: None,
        }
    }
}

impl MetricEvaluator for QuadraticMetric {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, _scenario: &ScenarioRef) -> Result<(), EvaluatorError> {
        self.gain = None;
        Ok(())
    }

    fn observe(&mut self, sample: &TelemetrySample) -> Result<(), EvaluatorError> {
        let mut state = self.state.0.borrow_mut();
        state.metric_observe_count = state.metric_observe_count.wrapping_add(1);
        drop(state);
        self.gain = sample.values.get("gain").copied();
        Ok(())
    }

    fn finish(&mut self) -> Result<MetricValues, EvaluatorError> {
        let gain = self
            .gain
            .take()
            .ok_or_else(|| EvaluatorError::new("metric has no sample"))?;
        Ok(MetricValues {
            loss: (gain - 1.0).powi(2),
            control_effort: gain.abs() / 2.0,
            objectives: BTreeMap::from([("test.response".to_owned(), gain.abs())]),
        })
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        self.gain = None;
        Ok(())
    }
}

#[derive(Clone)]
pub struct SequenceStrategy {
    identity: ArtifactIdentity,
    values: Vec<f64>,
    pub views: ObservedViews,
}

pub type ObservedViews = Rc<RefCell<Vec<(Vec<String>, Vec<TrainingObservation>)>>>;

impl SequenceStrategy {
    pub fn new(values: Vec<f64>) -> Self {
        Self {
            identity: identity("strategy", &format!("sequence={values:?}")),
            values,
            views: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl ProposalStrategy for SequenceStrategy {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn propose(&self, context: &ProposalContext<'_>) -> Result<Option<Proposal>, ProposalError> {
        self.views.borrow_mut().push((
            context
                .training
                .scenarios
                .iter()
                .map(|scenario| scenario.id.clone())
                .collect(),
            context.training.history.to_vec(),
        ));
        let Some(value) = self
            .values
            .get(context.training.attempt_index as usize)
            .copied()
        else {
            return Ok(None);
        };
        let candidate = context
            .training
            .incumbent
            .with_parameter("gain", value)
            .map_err(|error| ProposalError::new(error.to_string()))?;
        Ok(Some(Proposal {
            candidate,
            reason: format!("training sequence selected gain {value}"),
        }))
    }
}

pub fn candidate(gain: f64) -> Candidate {
    Candidate::new(
        CandidateLineage {
            schema: "aviate-candidate-v1".to_owned(),
            base_preset_digest: Digest::from_bytes([7; 32]),
            plant_digest: Digest::from_bytes([8; 32]),
        },
        BTreeMap::from([("gain".to_owned(), gain), ("mode".to_owned(), 1.0)]),
    )
    .expect("candidate")
}

pub fn stage() -> SearchStage {
    SearchStage {
        id: "inner-loop".to_owned(),
        allowlist: BTreeMap::from([(
            "gain".to_owned(),
            ParameterBounds {
                minimum: 0.0,
                maximum: 2.0,
            },
        )]),
        fixed_parameters: BTreeMap::from([("mode".to_owned(), 1.0)]),
        required_hard_gates: vec!["envelope".to_owned()],
        training_scenarios: vec![scenario("training-calm", 1)],
        promotion_scenarios: vec![scenario("promotion-gust", 2)],
        final_qualification_scenarios: vec![scenario("final-crosswind", 3)],
        repetitions: 2,
        promotion: PromotionPolicy {
            minimum_loss_improvement: 0.0,
            minimum_relative_loss_improvement: 0.2,
            maximum_control_effort_increase: 1.0,
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 0.5,
            maximum_p95_loss: 0.5,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::from([("test.response".to_owned(), 0.75)]),
        },
    }
}

fn scenario(id: &str, digest_byte: u8) -> ScenarioRef {
    ScenarioRef {
        id: id.to_owned(),
        digest: Digest::from_bytes([digest_byte; 32]),
        max_samples: 8,
        sample_timeout_ms: 100,
    }
}

fn identity(id: &str, content: &str) -> ArtifactIdentity {
    ArtifactIdentity::from_text(id, content).expect("artifact identity")
}

pub fn assert_receipt_error(error: TuneError) {
    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
}

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use flight_tune::{
    ArtifactIdentity, Candidate, CandidateLineage, Digest, EvaluatorError, GateEvaluator,
    GateOutcome, MetricEvaluator, MetricValues, ParameterBounds, PromotionPolicy, Proposal,
    ProposalContext, ProposalError, ProposalStrategy, QualificationPolicy, ScenarioRef,
    SearchStage, TelemetrySample, TrainingObservation, TuneError,
};

use super::{FakeHandle, FakeState, identity};

pub struct EnvelopeGates {
    identity: ArtifactIdentity,
    limit: f64,
    state: Option<FakeHandle>,
}

impl EnvelopeGates {
    pub fn new(limit: f64) -> Self {
        Self {
            identity: identity("gates", &format!("envelope-limit={limit}")),
            limit,
            state: None,
        }
    }

    #[allow(dead_code)]
    pub fn tracked(limit: f64, state: FakeHandle) -> Self {
        Self {
            identity: identity("gates", &format!("envelope-limit={limit}")),
            limit,
            state: Some(state),
        }
    }

    fn record(&self, update: impl FnOnce(&mut FakeState)) {
        if let Some(state) = &self.state {
            update(&mut state.0.borrow_mut());
        }
    }
}

impl GateEvaluator for EnvelopeGates {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, _scenario: &ScenarioRef) -> Result<(), EvaluatorError> {
        self.record(|state| {
            state.gate_begin_count = state.gate_begin_count.wrapping_add(1);
        });
        Ok(())
    }

    fn evaluate(&mut self, sample: &TelemetrySample) -> Result<Vec<GateOutcome>, EvaluatorError> {
        self.record(|state| {
            state.gate_evaluate_count = state.gate_evaluate_count.wrapping_add(1);
        });
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
        self.record(|state| {
            state.gate_finish_count = state.gate_finish_count.wrapping_add(1);
        });
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        self.record(|state| {
            state.gate_cancel_count = state.gate_cancel_count.wrapping_add(1);
        });
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
        let mut state = self.state.0.borrow_mut();
        state.metric_begin_count = state.metric_begin_count.wrapping_add(1);
        drop(state);
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
        let mut state = self.state.0.borrow_mut();
        state.metric_finish_count = state.metric_finish_count.wrapping_add(1);
        drop(state);
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
        let mut state = self.state.0.borrow_mut();
        state.metric_cancel_count = state.metric_cancel_count.wrapping_add(1);
        drop(state);
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
            schema: "generic-candidate-v1".to_owned(),
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

pub fn assert_receipt_error(error: TuneError) {
    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
}

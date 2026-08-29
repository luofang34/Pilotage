use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use flight_tune::{
    ArtifactIdentity, Candidate, CandidateLineage, Digest, EvaluatorError, GateEvaluator,
    GateOutcome, MetricEvaluator, MetricValues, MissionReference, ParameterBounds, PromotionPolicy,
    Proposal, ProposalContext, ProposalError, ProposalStrategy, QualificationPolicy, SearchStage,
    TelemetrySample, TrainingObservation, TuneError,
};

use super::{FAKE_MAX_SAMPLES, FakeHandle, FakeState, identity};

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

    fn begin(&mut self, _scenario: &MissionReference) -> Result<(), EvaluatorError> {
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

    fn begin(&mut self, _scenario: &MissionReference) -> Result<(), EvaluatorError> {
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
                .training_suites
                .iter()
                .flat_map(|suite| suite.primary_scenarios.iter().chain(&suite.guard_scenarios))
                .map(|scenario| scenario.revision_id.clone())
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

/// One suite that names every training mission the stage declares.
pub fn single_suite(training: &[MissionReference]) -> flight_tune::TrainingSuite {
    flight_tune::TrainingSuite {
        schema_version: flight_tune::TRAINING_SUITE_SCHEMA_VERSION,
        id: "inner-loop-suite".to_owned(),
        primary_scenarios: training.to_vec(),
        guard_scenarios: Vec::new(),
        guard_regression_limits: BTreeMap::new(),
        repetitions: 2,
    }
}

/// The one group every allowlisted parameter of the rig stage belongs to.
pub fn single_group() -> flight_tune::SearchGroup {
    flight_tune::SearchGroup {
        id: "inner-loop-group".to_owned(),
        kind: flight_tune::SearchGroupKind::Controller,
        parameters: BTreeSet::from(["gain".to_owned()]),
        suite_id: "inner-loop-suite".to_owned(),
    }
}

pub fn stage() -> SearchStage {
    let training = vec![scenario(super::FAKE_MISSION_IDS[0])];
    SearchStage {
        execution_retry: flight_tune::ExecutionRetryPolicy::none(),
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
        training_suites: vec![single_suite(&training)],
        search_groups: vec![single_group()],
        training_scenarios: training,
        promotion_scenarios: vec![scenario(super::FAKE_MISSION_IDS[1])],
        final_qualification_scenarios: vec![scenario(super::FAKE_MISSION_IDS[2])],
        repetitions: 2,
        promotion: PromotionPolicy {
            schema_version: flight_tune::PROMOTION_POLICY_SCHEMA_VERSION,
            seed_policy: flight_tune::PromotionSeedPolicy::PairedScenarioDigestV1,
            minimum_loss_improvement: 0.0,
            minimum_relative_loss_improvement: 0.2,
            maximum_control_effort_increase: 1.0,
            objective_regression_upper_95: BTreeMap::from([("test.response".to_owned(), 1.0)]),
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 0.5,
            maximum_p95_loss: 0.5,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::from([("test.response".to_owned(), 0.75)]),
        },
    }
}

fn scenario(id: &str) -> MissionReference {
    MissionReference::from_document(&super::fake_mission_document(id), FAKE_MAX_SAMPLES)
        .expect("mission reference")
}

/// The same stage, with the stated number of replacement executions allowed.
pub fn stage_with_execution_retry_limit(limit: u32) -> SearchStage {
    SearchStage {
        execution_retry: flight_tune::ExecutionRetryPolicy::with_limit(limit)
            .expect("a supported execution retry limit"),
        ..stage()
    }
}

/// The same stage with a training mission that commands one control family.
pub fn stage_with_stimulus_family(family: flight_tune::ControlFamily) -> SearchStage {
    let training = MissionReference::from_document(
        &super::fake_stimulus_mission_document(super::FAKE_MISSION_IDS[0], family),
        FAKE_MAX_SAMPLES,
    )
    .expect("stimulus mission reference");
    SearchStage {
        training_suites: vec![single_suite(std::slice::from_ref(&training))],
        training_scenarios: vec![training],
        ..stage()
    }
}

/// The same stage with one changed frozen suite declaration.
pub fn stage_with_changed_suite() -> SearchStage {
    let mut suites = vec![single_suite(&[scenario(super::FAKE_MISSION_IDS[0])])];
    suites[0].repetitions = 3;
    SearchStage {
        training_suites: suites,
        ..stage()
    }
}

/// The same stage with one changed training mission document.
pub fn stage_with_changed_training_mission() -> SearchStage {
    let changed = MissionReference::from_document(
        &super::changed_fake_mission_document(super::FAKE_MISSION_IDS[0]),
        FAKE_MAX_SAMPLES,
    )
    .expect("changed mission reference");
    SearchStage {
        training_suites: vec![single_suite(std::slice::from_ref(&changed))],
        training_scenarios: vec![changed],
        ..stage()
    }
}

pub fn assert_receipt_error(error: TuneError) {
    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
}

/// A candidate with the two parameters that the two-group stage searches.
pub fn two_group_candidate(gain: f64, trim: f64) -> Candidate {
    Candidate::new(
        CandidateLineage {
            schema: "generic-candidate-v1".to_owned(),
            base_preset_digest: Digest::from_bytes([7; 32]),
            plant_digest: Digest::from_bytes([8; 32]),
        },
        BTreeMap::from([("gain".to_owned(), gain), ("trim".to_owned(), trim)]),
    )
    .expect("two group candidate")
}

/// A stage whose two parameter groups take two different training suites.
///
/// The gain group is answered by a direct response suite. The trim group is
/// answered by an operator suite that guards the direct response, so a trim
/// change has to keep the response the gain group was tuned for.
pub fn two_group_stage() -> SearchStage {
    let direct = scenario(super::FAKE_MISSION_IDS[0]);
    let operator = scenario(super::FAKE_MISSION_IDS[3]);
    SearchStage {
        training_scenarios: vec![direct.clone(), operator.clone()],
        training_suites: vec![
            flight_tune::TrainingSuite {
                schema_version: flight_tune::TRAINING_SUITE_SCHEMA_VERSION,
                id: "direct-response".to_owned(),
                primary_scenarios: vec![direct.clone()],
                guard_scenarios: Vec::new(),
                guard_regression_limits: BTreeMap::new(),
                repetitions: 2,
            },
            flight_tune::TrainingSuite {
                schema_version: flight_tune::TRAINING_SUITE_SCHEMA_VERSION,
                id: "operator-feel".to_owned(),
                primary_scenarios: vec![operator],
                guard_scenarios: vec![direct],
                guard_regression_limits: BTreeMap::from([("test.response".to_owned(), 1.0)]),
                repetitions: 2,
            },
        ],
        search_groups: vec![
            flight_tune::SearchGroup {
                id: "gain-group".to_owned(),
                kind: flight_tune::SearchGroupKind::Controller,
                parameters: BTreeSet::from(["gain".to_owned()]),
                suite_id: "direct-response".to_owned(),
            },
            flight_tune::SearchGroup {
                id: "trim-group".to_owned(),
                kind: flight_tune::SearchGroupKind::OperatorFeel,
                parameters: BTreeSet::from(["trim".to_owned()]),
                suite_id: "operator-feel".to_owned(),
            },
        ],
        allowlist: BTreeMap::from([
            (
                "gain".to_owned(),
                ParameterBounds {
                    minimum: 0.0,
                    maximum: 2.0,
                },
            ),
            (
                "trim".to_owned(),
                ParameterBounds {
                    minimum: 0.0,
                    maximum: 2.0,
                },
            ),
        ]),
        fixed_parameters: BTreeMap::new(),
        ..stage()
    }
}

/// A strategy that states which parameters each proposal changes.
///
/// A proposal that changes two parameters from two groups is what proves the
/// engine refuses it, so the strategy has to be able to state one.
pub struct ParameterSequenceStrategy {
    identity: ArtifactIdentity,
    steps: Vec<Vec<(String, f64)>>,
}

impl ParameterSequenceStrategy {
    pub fn new(steps: Vec<Vec<(&str, f64)>>) -> Self {
        let owned = steps
            .into_iter()
            .map(|step| {
                step.into_iter()
                    .map(|(name, value)| (name.to_owned(), value))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        Self {
            identity: identity("strategy", &format!("parameter-sequence={owned:?}")),
            steps: owned,
        }
    }
}

impl ProposalStrategy for ParameterSequenceStrategy {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn propose(&self, context: &ProposalContext<'_>) -> Result<Option<Proposal>, ProposalError> {
        let Some(step) = self.steps.get(context.training.attempt_index as usize) else {
            return Ok(None);
        };
        let mut candidate = context.training.incumbent.clone();
        for (name, value) in step {
            candidate = candidate
                .with_parameter(name, *value)
                .map_err(|error| ProposalError::new(error.to_string()))?;
        }
        Ok(Some(Proposal {
            candidate,
            reason: format!("parameter sequence step {step:?}"),
        }))
    }
}

use crate::journal::AttemptRole;
use crate::{
    CandidateEvaluation, CandidateReceipt, Digest, GateEvaluator, Journal, MetricEvaluator,
    MetricValues, OperationStatus, RunRecord, ScenarioRef, ScenarioSet, ScenarioStartReceipt,
    SearchStage, SimulatorBackend, SimulatorCapability, TelemetrySample, TuneError,
};

pub(super) struct RunContext<'a> {
    pub(super) set: ScenarioSet,
    pub(super) scenario: &'a ScenarioRef,
    pub(super) repetition: u32,
    pub(super) seed: u64,
}

pub(super) enum RunTerminal {
    Passed {
        values: MetricValues,
        stop: OperationStatus,
    },
    HardGate {
        failure: crate::HardGateFailure,
    },
    Failed {
        error: TuneError,
        started: bool,
    },
}

pub(super) fn validate_sample(
    sample: &TelemetrySample,
    expected_sequence: u64,
    prior_elapsed_ms: u64,
) -> Result<(), TuneError> {
    if sample.sequence != expected_sequence
        || sample.elapsed_ms < prior_elapsed_ms
        || sample.values.is_empty()
    {
        return Err(TuneError::InvalidScore {
            detail: "telemetry sequence, time, or field set is not valid".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn begin_evaluators<G, M>(
    scenario: &ScenarioRef,
    gates: &mut G,
    metric: &mut M,
) -> Result<(), TuneError>
where
    G: GateEvaluator,
    M: MetricEvaluator,
{
    gates
        .begin(scenario)
        .map_err(|source| evaluator_error(gates.identity(), "begin hard gates", source))?;
    metric
        .begin(scenario)
        .map_err(|source| evaluator_error(metric.identity(), "begin metric", source))
}

pub(super) fn validate_candidate_receipt(
    receipt: Result<CandidateReceipt, crate::AdapterError>,
    capability: &SimulatorCapability,
    expected: Digest,
) -> Result<(), TuneError> {
    let receipt = receipt.map_err(|source| TuneError::Adapter {
        adapter: "bound simulator vehicle".to_owned(),
        operation: "apply candidate",
        source,
    })?;
    if receipt.session_digest != capability.session_digest()
        || receipt.requested_digest != expected
        || receipt.applied_digest != expected
        || receipt.readback_digest != expected
    {
        return Err(TuneError::ReceiptMismatch {
            operation: "apply candidate",
            detail: "applied or readback candidate digest does not match".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn validate_scenario_receipt(
    receipt: ScenarioStartReceipt,
    capability: &SimulatorCapability,
    context: &RunContext<'_>,
) -> Result<(), TuneError> {
    if receipt.session_digest != capability.session_digest()
        || receipt.applied_scenario_digest != context.scenario.digest
        || receipt.seed != context.seed
    {
        return Err(TuneError::ReceiptMismatch {
            operation: "start scenario",
            detail: "applied scenario digest or seed does not match".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn training_selection(
    journal: &Journal,
    role: AttemptRole,
    evaluation: &CandidateEvaluation,
) -> Option<bool> {
    match role {
        AttemptRole::TrainingBaseline => Some(evaluation.aggregate().is_some()),
        AttemptRole::TrainingChallenger { .. } => {
            Some(evaluation.aggregate().is_some_and(|challenger| {
                journal
                    .state()
                    .training_incumbent_evaluation
                    .as_ref()
                    .and_then(CandidateEvaluation::aggregate)
                    .is_none_or(|incumbent| challenger.mean_loss < incumbent.mean_loss)
            }))
        }
        _ => None,
    }
}

pub(super) fn run_record(
    stage: &SearchStage,
    context: &RunContext<'_>,
    values: MetricValues,
) -> RunRecord {
    RunRecord {
        scenario_set: context.set,
        scenario_id: context.scenario.id.clone(),
        repetition: context.repetition,
        seed: context.seed,
        loss: values.loss,
        control_effort: values.control_effort,
        objectives: values.objectives,
        passed_hard_gates: stage.required_hard_gates.clone(),
    }
}

pub(super) fn scenarios(stage: &SearchStage, set: ScenarioSet) -> &[ScenarioRef] {
    match set {
        ScenarioSet::Training => &stage.training_scenarios,
        ScenarioSet::Promotion => &stage.promotion_scenarios,
        ScenarioSet::FinalQualification => &stage.final_qualification_scenarios,
    }
}

pub(super) fn adapter_error<B: SimulatorBackend>(
    backend: &B,
    operation: &'static str,
    source: crate::AdapterError,
) -> TuneError {
    TuneError::Adapter {
        adapter: backend.simulator_identity().id.clone(),
        operation,
        source,
    }
}

pub(super) fn evaluator_error(
    identity: &crate::ArtifactIdentity,
    operation: &'static str,
    source: crate::EvaluatorError,
) -> TuneError {
    TuneError::Evaluator {
        implementation: identity.id.clone(),
        operation,
        source,
    }
}

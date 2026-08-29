use crate::journal::AttemptRole;
use crate::model::{AttemptRunPlan, derive_seed};
use crate::{
    CandidateEvaluation, HardGateFailure, MissionReference, RunRecord, ScenarioSet, SearchStage,
    TuneError,
};

#[cfg(test)]
#[path = "plan/tests.rs"]
mod tests;

pub(crate) fn validate_evaluation(
    evaluation: &CandidateEvaluation,
    role: AttemptRole,
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    let set = role.scenario_set();
    evaluation.validate(set)?;
    let plan = AttemptRunPlan::new(stage, role)?;
    let expected_count = plan.run_count();
    match evaluation {
        CandidateEvaluation::Passed { runs, .. } => {
            if runs.len() != expected_count {
                return Err(invalid("a passing evaluation has an incomplete run plan"));
            }
            validate_run_prefix(runs, set, &plan, stage, fixed_seed)
        }
        CandidateEvaluation::HardGateFailed {
            failure,
            completed_runs,
        } => {
            if completed_runs.len() >= expected_count {
                return Err(invalid("a hard gate failure has no remaining planned run"));
            }
            validate_run_prefix(completed_runs, set, &plan, stage, fixed_seed)?;
            validate_failure(failure, completed_runs.len(), set, &plan, stage, fixed_seed)
        }
        CandidateEvaluation::Quarantined { .. } => Ok(()),
    }
}

fn validate_run_prefix(
    runs: &[RunRecord],
    set: ScenarioSet,
    plan: &AttemptRunPlan,
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    for (index, run) in runs.iter().enumerate() {
        let (scenario, repetition) = plan.run_at(index)?;
        if run.scenario_set != set
            || run.mission_revision_id != scenario.revision_id
            || run.repetition != repetition
            || run.seed != derive_seed(fixed_seed, set, scenario, repetition)
            || run.passed_hard_gates != stage.required_hard_gates
        {
            return Err(invalid("a saved run does not match the prepared run plan"));
        }
    }
    Ok(())
}

fn validate_failure(
    failure: &HardGateFailure,
    run_index: usize,
    set: ScenarioSet,
    plan: &AttemptRunPlan,
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    let (scenario, repetition) = plan.run_at(run_index)?;
    let gate_is_required = stage
        .required_hard_gates
        .iter()
        .any(|gate| gate == &failure.gate.id);
    let gate_is_valid = core_gate_is_valid(failure, scenario)
        .unwrap_or(gate_is_required && failure.sample_sequence < u64::from(scenario.max_samples));
    if failure.scenario_set != set
        || failure.mission_revision_id != scenario.revision_id
        || failure.repetition != repetition
        || failure.seed != derive_seed(fixed_seed, set, scenario, repetition)
        || failure.gate.detail.trim().is_empty()
        || !gate_is_valid
    {
        return Err(invalid(
            "a hard gate failure does not match the prepared run plan",
        ));
    }
    Ok(())
}

fn core_gate_is_valid(failure: &HardGateFailure, scenario: &MissionReference) -> Option<bool> {
    match failure.gate.id.as_str() {
        "core.no_samples" => Some(failure.sample_sequence == 0 && failure.elapsed_ms == 0),
        "core.sample_limit" => Some(failure.sample_sequence == u64::from(scenario.max_samples)),
        "core.sample_timeout" => Some(failure.sample_sequence <= u64::from(scenario.max_samples)),
        _ => None,
    }
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}

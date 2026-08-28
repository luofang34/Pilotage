use crate::journal::AttemptRole;
use crate::model::derive_seed;
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
    let scenarios = scenarios(stage, set);
    let expected_count = scenarios.len() * stage.repetitions as usize;
    match evaluation {
        CandidateEvaluation::Passed { runs, .. } => {
            if runs.len() != expected_count {
                return Err(invalid("a passing evaluation has an incomplete run plan"));
            }
            validate_run_prefix(runs, set, scenarios, stage, fixed_seed)
        }
        CandidateEvaluation::HardGateFailed {
            failure,
            completed_runs,
        } => {
            if completed_runs.len() >= expected_count {
                return Err(invalid("a hard gate failure has no remaining planned run"));
            }
            validate_run_prefix(completed_runs, set, scenarios, stage, fixed_seed)?;
            validate_failure(
                failure,
                completed_runs.len(),
                set,
                scenarios,
                stage,
                fixed_seed,
            )
        }
        CandidateEvaluation::Quarantined { .. } => Ok(()),
    }
}

fn validate_run_prefix(
    runs: &[RunRecord],
    set: ScenarioSet,
    scenarios: &[MissionReference],
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    for (index, run) in runs.iter().enumerate() {
        let (scenario, repetition) = expected_run(scenarios, stage.repetitions, index)?;
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
    scenarios: &[MissionReference],
    stage: &SearchStage,
    fixed_seed: u64,
) -> Result<(), TuneError> {
    let (scenario, repetition) = expected_run(scenarios, stage.repetitions, run_index)?;
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

fn expected_run(
    scenarios: &[MissionReference],
    repetitions: u32,
    index: usize,
) -> Result<(&MissionReference, u32), TuneError> {
    let repetition_count = repetitions as usize;
    let scenario = scenarios
        .get(index / repetition_count)
        .ok_or_else(|| invalid("a saved run index exceeds the prepared run plan"))?;
    let repetition = u32::try_from(index % repetition_count)
        .map_err(|_| invalid("a saved repetition exceeds u32"))?;
    Ok((scenario, repetition))
}

fn scenarios(stage: &SearchStage, set: ScenarioSet) -> &[MissionReference] {
    match set {
        ScenarioSet::Training => &stage.training_scenarios,
        ScenarioSet::Promotion => &stage.promotion_scenarios,
        ScenarioSet::FinalQualification => &stage.final_qualification_scenarios,
    }
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}

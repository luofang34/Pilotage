use crate::score::paired_stats;
use crate::{CandidateEvaluation, PromotionDecision, PromotionPolicy, RunRecord, TuneError};

pub(super) fn decide(
    policy: PromotionPolicy,
    baseline: Option<&CandidateEvaluation>,
    frozen: Option<&CandidateEvaluation>,
) -> Result<PromotionDecision, TuneError> {
    if let Some(gate_id) = [baseline, frozen]
        .into_iter()
        .flatten()
        .find_map(gate_failure)
    {
        return Ok(PromotionDecision::RejectedHardGate { gate_id });
    }
    if [baseline, frozen].into_iter().flatten().any(is_quarantined) {
        return Ok(PromotionDecision::Indeterminate {
            reason: "promotion quarantined one frozen evaluation".to_owned(),
        });
    }
    let (Some(baseline), Some(frozen)) = (baseline, frozen) else {
        return Ok(PromotionDecision::Indeterminate {
            reason: "promotion did not complete both frozen evaluations".to_owned(),
        });
    };
    let (baseline_runs, frozen_runs) = passing_runs(baseline, frozen)?;
    validate_pairs(baseline_runs, frozen_runs)?;
    let loss = paired_stats(
        baseline_runs
            .iter()
            .zip(frozen_runs)
            .map(|(baseline, frozen)| frozen.loss - baseline.loss),
    )?;
    let effort = paired_stats(
        baseline_runs
            .iter()
            .zip(frozen_runs)
            .map(|(baseline, frozen)| frozen.control_effort - baseline.control_effort),
    )?;
    if loss.upper_95 <= -policy.minimum_loss_improvement
        && effort.mean <= policy.maximum_control_effort_increase
    {
        return Ok(PromotionDecision::Promoted {
            mean_loss_delta: loss.mean,
            loss_delta_upper_95: loss.upper_95,
            mean_effort_delta: effort.mean,
        });
    }
    Ok(PromotionDecision::RejectedNoImprovement {
        loss_delta_upper_95: loss.upper_95,
        mean_effort_delta: effort.mean,
    })
}

fn gate_failure(evaluation: &CandidateEvaluation) -> Option<String> {
    if let CandidateEvaluation::HardGateFailed { failure, .. } = evaluation {
        Some(failure.gate.id.clone())
    } else {
        None
    }
}

fn is_quarantined(evaluation: &CandidateEvaluation) -> bool {
    matches!(evaluation, CandidateEvaluation::Quarantined { .. })
}

fn passing_runs<'a>(
    baseline: &'a CandidateEvaluation,
    frozen: &'a CandidateEvaluation,
) -> Result<(&'a [RunRecord], &'a [RunRecord]), TuneError> {
    match (baseline, frozen) {
        (
            CandidateEvaluation::Passed {
                runs: baseline_runs,
                ..
            },
            CandidateEvaluation::Passed {
                runs: frozen_runs, ..
            },
        ) => Ok((baseline_runs, frozen_runs)),
        _ => Err(TuneError::InvalidScore {
            detail: "promotion expected two passing evaluations".to_owned(),
        }),
    }
}

fn validate_pairs(baseline: &[RunRecord], frozen: &[RunRecord]) -> Result<(), TuneError> {
    if baseline.len() != frozen.len() || baseline.len() < 2 {
        return Err(pair_error());
    }
    for (left, right) in baseline.iter().zip(frozen) {
        if left.scenario_set != right.scenario_set
            || left.scenario_id != right.scenario_id
            || left.repetition != right.repetition
            || left.seed != right.seed
        {
            return Err(pair_error());
        }
    }
    Ok(())
}

fn pair_error() -> TuneError {
    TuneError::InvalidScore {
        detail: "promotion run keys do not form exact pairs".to_owned(),
    }
}

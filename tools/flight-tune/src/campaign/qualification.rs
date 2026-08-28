//! Final qualification policy evaluation.

use crate::{CandidateEvaluation, FinalQualificationOutcome, SearchStage};

pub(crate) fn final_outcome(
    stage: &SearchStage,
    evaluation: Option<&CandidateEvaluation>,
) -> FinalQualificationOutcome {
    match evaluation {
        Some(CandidateEvaluation::Passed { aggregate, runs }) => {
            passed_outcome(stage, aggregate, runs)
        }
        Some(CandidateEvaluation::HardGateFailed { failure, .. }) => {
            FinalQualificationOutcome::FailedHardGate {
                gate_id: failure.gate.id.clone(),
            }
        }
        Some(CandidateEvaluation::Quarantined { reason }) => {
            FinalQualificationOutcome::Indeterminate {
                reason: reason.clone(),
            }
        }
        None => FinalQualificationOutcome::Indeterminate {
            reason: "final qualification did not complete".to_owned(),
        },
    }
}

fn passed_outcome(
    stage: &SearchStage,
    aggregate: &crate::ScoreAggregate,
    runs: &[crate::RunRecord],
) -> FinalQualificationOutcome {
    let policy = &stage.qualification;
    let failed = [
        (
            "loss_confidence_95.upper",
            aggregate.loss_confidence_95.upper,
            policy.maximum_loss_confidence_upper,
        ),
        ("p95_loss", aggregate.p95_loss, policy.maximum_p95_loss),
        (
            "mean_control_effort",
            aggregate.mean_control_effort,
            policy.maximum_mean_control_effort,
        ),
    ]
    .into_iter()
    .find(|(_, actual, maximum)| actual > maximum);
    if let Some((metric, _, _)) = failed {
        return FinalQualificationOutcome::FailedObjective {
            metric: metric.to_owned(),
        };
    }
    for (metric, maximum) in &policy.objective_maxima {
        let values = runs
            .iter()
            .filter_map(|run| run.objectives.get(metric).copied())
            .collect::<Vec<_>>();
        if values.len() != runs.len() || values.iter().any(|value| value > maximum) {
            return FinalQualificationOutcome::FailedObjective {
                metric: metric.clone(),
            };
        }
    }
    FinalQualificationOutcome::Qualified
}

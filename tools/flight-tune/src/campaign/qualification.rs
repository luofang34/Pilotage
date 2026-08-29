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
    scoped_outcome(stage, runs)
}

/// Every final run against the limit its own scenario states.
///
/// A run that names a scenario with no scoped row, or that states no value for
/// a declared objective, fails the objective rather than passing it. There is
/// no global maximum to fall back on, and a decision that cannot find its bar
/// is not a decision that met one.
fn scoped_outcome(stage: &SearchStage, runs: &[crate::RunRecord]) -> FinalQualificationOutcome {
    for run in runs {
        let resolved = run
            .objectives
            .get(crate::TARGET_AUTHORITY_OBJECTIVE)
            .copied();
        if !stage
            .response_targets
            .authority_holds(&run.mission_revision_id, resolved)
        {
            return FinalQualificationOutcome::FailedObjective {
                metric: crate::TARGET_AUTHORITY_OBJECTIVE.to_owned(),
            };
        }
    }
    for metric in &stage.qualification.objectives {
        for run in runs {
            let within = stage
                .response_targets
                .target(&run.mission_revision_id, metric)
                .ok()
                .zip(run.objectives.get(metric).copied())
                .is_some_and(|(target, value)| target.holds(value));
            if !within {
                return FinalQualificationOutcome::FailedObjective {
                    metric: metric.clone(),
                };
            }
        }
    }
    FinalQualificationOutcome::Qualified
}

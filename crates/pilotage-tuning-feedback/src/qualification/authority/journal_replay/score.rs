//! Scores the independent replay derives instead of reading.

use flight_tune::CandidateEvaluation;

use crate::{FeedbackError, error::invalid};

use super::super::super::evaluation as evaluation_rules;
use super::super::super::statistics;

/// Derives the mean loss one completed evaluation states.
///
/// The training incumbent decision turns on this value, so reading it back
/// out of the document would make the document its own authority. It is
/// calculated again from the run records the same evaluation carries, with
/// the core algorithm, and the stored aggregate must equal it field for
/// field.
pub(super) fn derived_mean_loss(
    evaluation: &CandidateEvaluation,
) -> Result<Option<f64>, FeedbackError> {
    match evaluation {
        CandidateEvaluation::Passed { aggregate, runs } => {
            for run in runs {
                evaluation_rules::verify_objectives(&run.objectives)?;
            }
            let derived = statistics::aggregate(runs)?;
            if aggregate != &derived {
                return Err(invalid(
                    "a completed evaluation aggregate changed from its run records",
                ));
            }
            Ok(Some(derived.mean_loss))
        }
        CandidateEvaluation::HardGateFailed { completed_runs, .. } => {
            for run in completed_runs {
                evaluation_rules::verify_objectives(&run.objectives)?;
            }
            Ok(None)
        }
        CandidateEvaluation::Quarantined { .. } => Ok(None),
    }
}

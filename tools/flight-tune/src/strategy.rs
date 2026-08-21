use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use pilotage_trial::Digest;

use crate::{ArtifactIdentity, Candidate, ParameterBounds, ScenarioRef};

/// A prior training result that a proposal strategy can inspect.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainingObservation {
    /// The zero-based training challenger index.
    pub attempt_index: u64,
    /// The challenger candidate digest.
    pub candidate_digest: Digest,
    /// Whether training selected this challenger as the new incumbent.
    pub selected_as_incumbent: bool,
    /// Whether a hard gate stopped this challenger.
    pub hard_gate_failed: bool,
    /// Whether recovery or an execution error quarantined this challenger.
    pub quarantined: bool,
    /// The training mean loss when all hard gates passed.
    pub training_mean_loss: Option<f64>,
}

/// The only campaign data available to an adaptive proposal strategy.
#[derive(Debug)]
pub struct TrainingView<'a> {
    /// The fixed tuning-session seed.
    pub fixed_seed: u64,
    /// The zero-based training challenger index.
    pub attempt_index: u64,
    /// The stable stage name.
    pub stage_id: &'a str,
    /// The only parameters that a proposal can change.
    pub allowlist: &'a BTreeMap<String, ParameterBounds>,
    /// The training scenarios.
    pub scenarios: &'a [ScenarioRef],
    /// The repeated run count for each training scenario.
    pub repetitions: u32,
    /// The current training incumbent.
    pub incumbent: &'a Candidate,
    /// Prior training challenger results in journal order.
    pub history: &'a [TrainingObservation],
}

/// Read-only input for one deterministic training proposal.
#[derive(Debug)]
pub struct ProposalContext<'a> {
    /// The isolated adaptive training view.
    pub training: TrainingView<'a>,
}

/// One proposed candidate and the reason for its selection.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// The complete proposed candidate.
    pub candidate: Candidate,
    /// A stable explanation of the proposal decision.
    pub reason: String,
}

/// An error from a proposal strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalError {
    detail: String,
}

impl ProposalError {
    /// Creates a proposal error.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ProposalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for ProposalError {}

/// A replaceable deterministic candidate proposal strategy.
pub trait ProposalStrategy {
    /// Returns the strategy implementation and configuration identity.
    fn identity(&self) -> &ArtifactIdentity;

    /// Proposes the next training candidate or completes the search.
    ///
    /// Implementations must return the same result for the same training view.
    fn propose(&self, context: &ProposalContext<'_>) -> Result<Option<Proposal>, ProposalError>;
}

/// A deterministic bounded coordinate proposal strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundedCoordinateSearch {
    step_fraction: f64,
    identity: ArtifactIdentity,
}

impl BoundedCoordinateSearch {
    /// Creates a strategy with a fraction of each parameter range as its first
    /// step size.
    ///
    /// # Errors
    ///
    /// Returns [`ProposalError`] when the fraction is not in `(0, 1]`.
    pub fn new(step_fraction: f64) -> Result<Self, ProposalError> {
        if !(step_fraction.is_finite() && 0.0 < step_fraction && step_fraction <= 1.0) {
            return Err(ProposalError::new(
                "coordinate step fraction must be in (0, 1]",
            ));
        }
        let identity = ArtifactIdentity::from_text(
            "bounded-coordinate-search",
            &format!(
                "step_fraction={step_fraction}\nimplementation={}",
                include_str!("strategy.rs")
            ),
        )
        .map_err(|error| ProposalError::new(error.to_string()))?;
        Ok(Self {
            step_fraction,
            identity,
        })
    }
}

impl ProposalStrategy for BoundedCoordinateSearch {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn propose(&self, context: &ProposalContext<'_>) -> Result<Option<Proposal>, ProposalError> {
        let training = &context.training;
        let parameter_count = training.allowlist.len();
        if parameter_count == 0 {
            return Ok(None);
        }
        let pair_index = training.attempt_index / 2;
        let sweep = pair_index / parameter_count as u64;
        let seed_offset = (training.fixed_seed % parameter_count as u64) as usize;
        let coordinate = (pair_index as usize).wrapping_add(seed_offset) % parameter_count;
        let Some((name, bounds)) = training.allowlist.iter().nth(coordinate) else {
            return Ok(None);
        };
        let Some(current) = training.incumbent.parameters().get(name).copied() else {
            return Err(ProposalError::new(format!(
                "training incumbent does not contain {name}"
            )));
        };
        let positive_first = training.fixed_seed & 1 == 0;
        let first_direction = if positive_first { 1.0 } else { -1.0 };
        let direction = if training.attempt_index & 1 == 0 {
            first_direction
        } else {
            -first_direction
        };
        let divisor = sweep as f64 + 1.0;
        let step = (bounds.maximum - bounds.minimum) * self.step_fraction / divisor;
        let mut proposed = (current + direction * step).clamp(bounds.minimum, bounds.maximum);
        if proposed == current {
            proposed = (current - direction * step).clamp(bounds.minimum, bounds.maximum);
        }
        if proposed == current {
            return Ok(None);
        }
        let candidate = training
            .incumbent
            .with_parameter(name, proposed)
            .map_err(|error| ProposalError::new(error.to_string()))?;
        Ok(Some(Proposal {
            candidate,
            reason: format!(
                "seed {} selected {name}; attempt {} moved {current} to {proposed}",
                training.fixed_seed, training.attempt_index
            ),
        }))
    }
}

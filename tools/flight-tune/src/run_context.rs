use serde::{Deserialize, Serialize};

use crate::identity::digest_bytes;
use crate::{
    AttemptRole, CandidateTransitionReference, Digest, ScenarioRef, ScenarioSet, TuneError,
};

/// The supported run execution context schema.
pub const RUN_EXECUTION_CONTEXT_SCHEMA_VERSION: u16 = 1;

const RUN_CONTEXT_DOMAIN: &[u8] = b"flight-tune:run-execution-context:v1\0";

/// The immutable identity of one simulator run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunExecutionContext {
    schema_version: u16,
    tuning_session_digest: Digest,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
    transition_authorization: Option<CandidateTransitionReference>,
    scenario_set: ScenarioSet,
    scenario_id: String,
    scenario_digest: Digest,
    repetition: u32,
    seed: u64,
}

impl RunExecutionContext {
    /// Creates one complete run identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when an identity is incomplete or inconsistent.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tuning_session_digest: Digest,
        trial_id: u64,
        role: AttemptRole,
        candidate_digest: Digest,
        transition_authorization: Option<CandidateTransitionReference>,
        scenario_set: ScenarioSet,
        scenario: &ScenarioRef,
        repetition: u32,
        seed: u64,
    ) -> Result<Self, TuneError> {
        let context = Self {
            schema_version: RUN_EXECUTION_CONTEXT_SCHEMA_VERSION,
            tuning_session_digest,
            trial_id,
            role,
            candidate_digest,
            transition_authorization,
            scenario_set,
            scenario_id: scenario.id.clone(),
            scenario_digest: scenario.digest,
            repetition,
            seed,
        };
        context.validate()?;
        Ok(context)
    }

    /// Validates the structure and internal consistency of this run identity.
    ///
    /// This method does not authorize a candidate transition. Journal replay
    /// must validate the transition reference against the frozen runtime
    /// contract.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when an identity is incomplete or inconsistent.
    pub fn validate(&self) -> Result<(), TuneError> {
        let transition_matches = match self.role {
            AttemptRole::TrainingChallenger { .. } => self
                .transition_authorization
                .is_some_and(|reference| reference.is_valid_for_target(self.candidate_digest)),
            AttemptRole::TrainingBaseline
            | AttemptRole::PromotionBaseline
            | AttemptRole::PromotionFrozen
            | AttemptRole::FinalQualification => self.transition_authorization.is_none(),
        };
        if self.schema_version != RUN_EXECUTION_CONTEXT_SCHEMA_VERSION
            || self.tuning_session_digest.is_zero()
            || self.candidate_digest.is_zero()
            || self.role.scenario_set() != self.scenario_set
            || !transition_matches
            || self.scenario_id.trim().is_empty()
            || self.scenario_id.len() > 128
            || self.scenario_digest.is_zero()
        {
            return Err(TuneError::InvalidIdentity {
                detail: "the run execution context is incomplete or inconsistent".to_owned(),
            });
        }
        Ok(())
    }

    /// Returns the canonical run intent identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when validation or encoding fails.
    pub fn digest(&self) -> Result<Digest, TuneError> {
        self.validate()?;
        let document = serde_json::to_vec(self).map_err(|source| TuneError::Encode {
            document: "run execution context",
            source,
        })?;
        let mut bytes = Vec::with_capacity(RUN_CONTEXT_DOMAIN.len().saturating_add(document.len()));
        bytes.extend_from_slice(RUN_CONTEXT_DOMAIN);
        bytes.extend_from_slice(&document);
        Ok(digest_bytes(&bytes))
    }

    /// Returns the tuning session identity.
    #[must_use]
    pub const fn tuning_session_digest(&self) -> Digest {
        self.tuning_session_digest
    }

    /// Returns the campaign trial identity.
    #[must_use]
    pub const fn trial_id(&self) -> u64 {
        self.trial_id
    }

    /// Returns the attempt role.
    #[must_use]
    pub const fn role(&self) -> AttemptRole {
        self.role
    }

    /// Returns the candidate identity.
    #[must_use]
    pub const fn candidate_digest(&self) -> Digest {
        self.candidate_digest
    }

    /// Returns the transition authorization for this run.
    #[must_use]
    pub const fn transition_authorization(&self) -> Option<CandidateTransitionReference> {
        self.transition_authorization
    }

    /// Returns the scenario partition.
    #[must_use]
    pub const fn scenario_set(&self) -> ScenarioSet {
        self.scenario_set
    }

    /// Returns the scenario name.
    #[must_use]
    pub fn scenario_id(&self) -> &str {
        &self.scenario_id
    }

    /// Returns the scenario artifact identity.
    #[must_use]
    pub const fn scenario_digest(&self) -> Digest {
        self.scenario_digest
    }

    /// Returns the zero-based scenario repetition.
    #[must_use]
    pub const fn repetition(&self) -> u32 {
        self.repetition
    }

    /// Returns the deterministic run seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }
}

#[cfg(test)]
mod tests;

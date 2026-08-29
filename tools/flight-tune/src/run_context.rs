use serde::{Deserialize, Serialize};

use crate::identity::digest_bytes;
use crate::{
    AttemptRole, CandidateTransitionReference, Digest, MissionReference, ScenarioSet, TuneError,
};

/// The supported run execution context schema.
pub const RUN_EXECUTION_CONTEXT_SCHEMA_VERSION: u16 = 3;

const RUN_CONTEXT_DOMAIN: &[u8] = b"flight-tune:run-execution-context:v3\0";

/// The immutable identity of one simulator run.
///
/// The seed is a pure function of the session, partition, mission, and
/// repetition, so two executions of one experimental condition differ in
/// nothing except the retry index. Without that field a replacement execution
/// would carry the identity of the execution it replaced.
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
    mission_revision_id: String,
    mission_content_digest: Digest,
    repetition: u32,
    seed: u64,
    retry_index: u32,
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
        mission: &MissionReference,
        repetition: u32,
        seed: u64,
        retry_index: u32,
    ) -> Result<Self, TuneError> {
        let context = Self {
            schema_version: RUN_EXECUTION_CONTEXT_SCHEMA_VERSION,
            tuning_session_digest,
            trial_id,
            role,
            candidate_digest,
            transition_authorization,
            scenario_set,
            mission_revision_id: mission.revision_id.clone(),
            mission_content_digest: mission.content_digest,
            repetition,
            seed,
            retry_index,
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
            || self.mission_revision_id.trim().is_empty()
            || self.mission_revision_id.len() > 128
            || self.mission_content_digest.is_zero()
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

    /// Returns the executed mission revision.
    #[must_use]
    pub fn mission_revision_id(&self) -> &str {
        &self.mission_revision_id
    }

    /// Returns the executed mission content identity.
    #[must_use]
    pub const fn mission_content_digest(&self) -> Digest {
        self.mission_content_digest
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

    /// Returns how many replacements separate this run from its first attempt.
    #[must_use]
    pub const fn retry_index(&self) -> u32 {
        self.retry_index
    }

    /// Reports whether two identities state one experimental condition.
    ///
    /// The trial identity and the retry index are the only fields a
    /// replacement execution may change.
    #[must_use]
    pub fn states_same_condition(&self, other: &Self) -> bool {
        self.tuning_session_digest == other.tuning_session_digest
            && self.role == other.role
            && self.candidate_digest == other.candidate_digest
            && self.transition_authorization == other.transition_authorization
            && self.scenario_set == other.scenario_set
            && self.mission_revision_id == other.mission_revision_id
            && self.mission_content_digest == other.mission_content_digest
            && self.repetition == other.repetition
            && self.seed == other.seed
    }
}

#[cfg(test)]
mod tests;

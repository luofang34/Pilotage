//! The vehicle-owned rule for moving between two candidates.
//!
//! A candidate is a command law the vehicle will actually fly. Two laws
//! that are each safe on their own are not automatically safe to step
//! between: the step happens on a flying aircraft, in one control frame.
//! The adjacency policy states how far one step may move each searched
//! parameter, and the validator checks the exact source and the exact
//! target against it before anything is written to a controller.
//!
//! Both the policy and the validator carry an explicit identity. The
//! runtime binds them, so a changed rule is a changed runtime identity, and
//! a suite baseline measured under one rule is not comparable evidence for
//! a challenger measured under another.
//!
//! SIM / NOT FOR FLIGHT.

use std::collections::BTreeMap;

use flight_tune::{
    AdapterError, ArtifactIdentity, Candidate, CandidateTransitionReceipt,
    CandidateTransitionRequest, Digest, TuneError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The supported adjacency policy document schema.
pub const ADJACENCY_POLICY_SCHEMA_VERSION: u16 = 1;

/// The stable name of the Aviate candidate-transition validator.
pub const TRANSITION_VALIDATOR_ID: &str = "pilotage-aviate-transition-validator-v1";

const VALIDATOR_DOMAIN: &[u8] = b"pilotage-aviate-transition-validator-v1\0";
const ADJACENCY_POLICY_DOMAIN: &[u8] = b"pilotage-aviate-adjacency-policy-v1\0";

/// The largest step one transition may take in one searched parameter.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParameterStepLimit {
    /// The largest absolute change, in the parameter's own unit.
    pub absolute: f64,
    /// The largest change as a fraction of the incumbent magnitude.
    pub fraction: f64,
}

impl ParameterStepLimit {
    /// Whether one change from an incumbent value is inside this limit.
    ///
    /// The two bounds are alternatives, not both: a parameter near zero
    /// needs the absolute bound, and a large parameter needs the
    /// proportional one. A step inside either is adjacent.
    #[must_use]
    pub fn permits(&self, from: f64, to: f64) -> bool {
        if !from.is_finite() || !to.is_finite() {
            return false;
        }
        let change = (to - from).abs();
        change <= self.absolute || change <= self.fraction * from.abs()
    }
}

/// How far one candidate transition may move each searched parameter.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdjacencyPolicy {
    /// Document schema version.
    pub schema_version: u16,
    /// The stable policy name.
    pub id: String,
    /// The largest step for each parameter a transition may change.
    pub limits: BTreeMap<String, ParameterStepLimit>,
}

impl AdjacencyPolicy {
    /// Creates one validated adjacency policy.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the name is empty, the policy states
    /// no parameter, or a limit is not a usable positive number.
    pub fn new(
        id: impl Into<String>,
        limits: BTreeMap<String, ParameterStepLimit>,
    ) -> Result<Self, AdapterError> {
        let policy = Self {
            schema_version: ADJACENCY_POLICY_SCHEMA_VERSION,
            id: id.into(),
            limits,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// The exact identity of this policy.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the policy is invalid or cannot encode.
    pub fn digest(&self) -> Result<Digest, AdapterError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|source| AdapterError::new(format!("adjacency policy: {source}")))?;
        let mut hasher = Sha256::new();
        hasher.update(ADJACENCY_POLICY_DOMAIN);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        Ok(Digest::from_bytes(hasher.finalize().into()))
    }

    /// Whether one exact transition is adjacent under this policy.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the candidates state different
    /// parameters, when a changed parameter has no declared limit, or when
    /// a step is larger than its limit permits.
    pub fn require_adjacent(
        &self,
        source: &Candidate,
        target: &Candidate,
    ) -> Result<(), AdapterError> {
        self.validate()?;
        let from = source.parameters();
        let to = target.parameters();
        if from.len() != to.len() || from.keys().ne(to.keys()) {
            return Err(AdapterError::new(
                "the transition changes the candidate parameter set",
            ));
        }
        for (name, target_value) in to {
            let Some(source_value) = from.get(name) else {
                return Err(AdapterError::new(format!(
                    "the transition adds the parameter {name}"
                )));
            };
            if source_value.to_bits() == target_value.to_bits() {
                continue;
            }
            let Some(limit) = self.limits.get(name) else {
                return Err(AdapterError::new(format!(
                    "the adjacency policy states no step limit for {name}"
                )));
            };
            if !limit.permits(*source_value, *target_value) {
                return Err(AdapterError::new(format!(
                    "the transition moves {name} further than one adjacent step"
                )));
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), AdapterError> {
        if self.schema_version != ADJACENCY_POLICY_SCHEMA_VERSION
            || self.id.trim().is_empty()
            || self.id.len() > 128
            || self.limits.is_empty()
        {
            return Err(AdapterError::new("the adjacency policy is incomplete"));
        }
        let usable = self.limits.values().all(|limit| {
            limit.absolute.is_finite()
                && limit.absolute > 0.0
                && limit.fraction.is_finite()
                && limit.fraction >= 0.0
        });
        if usable {
            Ok(())
        } else {
            Err(AdapterError::new(
                "an adjacency step limit is not a usable positive number",
            ))
        }
    }
}

/// The vehicle-owned candidate transition validator.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionValidator {
    policy: AdjacencyPolicy,
    policy_digest: Digest,
    identity: ArtifactIdentity,
}

impl TransitionValidator {
    /// Creates the validator for one adjacency policy.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the policy is invalid.
    pub fn new(policy: AdjacencyPolicy) -> Result<Self, AdapterError> {
        let policy_digest = policy.digest()?;
        let identity = validator_identity(policy_digest)
            .map_err(|source| AdapterError::new(source.to_string()))?;
        Ok(Self {
            policy,
            policy_digest,
            identity,
        })
    }

    /// The exact validator identity.
    #[must_use]
    pub const fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    /// The exact adjacency-policy identity.
    #[must_use]
    pub const fn policy_digest(&self) -> Digest {
        self.policy_digest
    }

    /// The adjacency policy this validator enforces.
    #[must_use]
    pub const fn policy(&self) -> &AdjacencyPolicy {
        &self.policy
    }

    /// Authorizes one exact transition without controller mutation.
    ///
    /// The request carries the current incumbent as its source, so a later
    /// transition is checked against the candidate that is actually
    /// active rather than against the one the search started from.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when the request names another validator or
    /// policy, or when the transition is not adjacent.
    pub fn authorize(
        &self,
        request: &CandidateTransitionRequest,
    ) -> Result<CandidateTransitionReceipt, AdapterError> {
        if request.validator() != &self.identity
            || request.adjacency_policy_digest() != self.policy_digest
        {
            return Err(AdapterError::new(
                "the transition request names another validator or adjacency policy",
            ));
        }
        self.policy
            .require_adjacent(request.source(), request.target())?;
        CandidateTransitionReceipt::authorized(request)
            .map_err(|source| AdapterError::new(source.to_string()))
    }
}

/// The content identity of this validator and its exact policy.
///
/// # Errors
///
/// Returns [`TuneError`] when the identity cannot be named.
pub fn validator_identity(policy_digest: Digest) -> Result<ArtifactIdentity, TuneError> {
    let source = include_bytes!("transition_authorization.rs");
    let mut hasher = Sha256::new();
    hasher.update(VALIDATOR_DOMAIN);
    hasher.update((source.len() as u64).to_le_bytes());
    hasher.update(source);
    hasher.update(policy_digest.as_bytes());
    ArtifactIdentity::new(
        TRANSITION_VALIDATOR_ID,
        Digest::from_bytes(hasher.finalize().into()),
    )
}

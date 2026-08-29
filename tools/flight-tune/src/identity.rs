use pilotage_trial::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as ShaDigest, Sha256};

use crate::TuneError;

const SCENARIO_RUNTIME_ID_V2: &str = "pilotage-scenario-runtime-v2";

/// The content identity of one runtime artifact or implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    /// The stable artifact name.
    pub id: String,
    /// The SHA-256 digest of the exact artifact or configuration.
    pub digest: Digest,
}

impl ArtifactIdentity {
    /// Creates a validated artifact identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the name is empty or the digest is zero.
    pub fn new(id: impl Into<String>, digest: Digest) -> Result<Self, TuneError> {
        let identity = Self {
            id: id.into(),
            digest,
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Creates an identity from exact text content.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the name is empty.
    pub fn from_text(id: impl Into<String>, text: &str) -> Result<Self, TuneError> {
        Self::new(id, digest_bytes(text.as_bytes()))
    }

    pub(crate) fn validate(&self) -> Result<(), TuneError> {
        if self.id.trim().is_empty() || self.id.len() > 256 || self.digest.is_zero() {
            return Err(TuneError::InvalidIdentity {
                detail: "an artifact identity needs a short name and a nonzero digest".to_owned(),
            });
        }
        Ok(())
    }
}

/// The exact evaluator implementations one session is frozen to.
///
/// The metric evaluator and the hard-gate evaluator have separate production
/// inventories, so the two identities cannot hold the same value. A pair that
/// does would let one evaluator stand in for the other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorIdentities {
    /// The continuous metric implementation and its configuration.
    pub metric: ArtifactIdentity,
    /// The streaming hard gate implementation and its configuration.
    pub hard_gates: ArtifactIdentity,
}

impl EvaluatorIdentities {
    /// Creates one validated evaluator identity pair.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one evaluator identity is not valid, or when
    /// the two identities cannot describe separate evaluators.
    pub fn new(metric: ArtifactIdentity, hard_gates: ArtifactIdentity) -> Result<Self, TuneError> {
        let identities = Self { metric, hard_gates };
        identities.validate()?;
        Ok(identities)
    }

    /// Rejects a pair that cannot describe two separate evaluators.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one evaluator identity is not valid, or when
    /// the two identities share a name or a digest.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.metric.validate()?;
        self.hard_gates.validate()?;
        if self.metric.id == self.hard_gates.id || self.metric.digest == self.hard_gates.digest {
            return Err(TuneError::EvaluatorIdentityChanged {
                detail: "one evaluator identity stands in for the other".to_owned(),
            });
        }
        Ok(())
    }

    /// Rejects evaluators that are not the ones the plan froze.
    ///
    /// A restart, a retry, a suite comparison, a promotion, and a final
    /// qualification all read the frozen plan first. This check runs before
    /// any of them, so a rebuilt evaluator cannot reach a simulator, a
    /// process, a vehicle, or a journal.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one pair is not valid, or when the two pairs
    /// differ.
    pub fn require_frozen(&self, frozen: &Self) -> Result<(), TuneError> {
        self.validate()?;
        frozen.validate()?;
        if self.metric != frozen.metric {
            return Err(TuneError::EvaluatorIdentityChanged {
                detail: "the metric evaluator is not the one the plan froze".to_owned(),
            });
        }
        if self.hard_gates != frozen.hard_gates {
            return Err(TuneError::EvaluatorIdentityChanged {
                detail: "the hard gate evaluator is not the one the plan froze".to_owned(),
            });
        }
        Ok(())
    }
}

/// The immutable source identity for all candidates in one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateLineage {
    /// The candidate document schema identity.
    pub schema: String,
    /// The digest of the immutable base preset.
    pub base_preset_digest: Digest,
    /// The digest of the plant identification artifact.
    pub plant_digest: Digest,
}

impl CandidateLineage {
    /// Validates the candidate lineage.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one identity is missing.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema.trim().is_empty()
            || self.schema.len() > 128
            || self.base_preset_digest.is_zero()
            || self.plant_digest.is_zero()
        {
            return Err(TuneError::InvalidIdentity {
                detail: "candidate lineage is incomplete".to_owned(),
            });
        }
        Ok(())
    }
}

/// All executable and plant identities for one tuning session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeIdentities {
    /// The tuning harness build.
    pub harness_build: ArtifactIdentity,
    /// The proposal strategy and its exact configuration.
    pub strategy: ArtifactIdentity,
    /// The continuous metric implementation and its configuration.
    pub metric: ArtifactIdentity,
    /// The streaming hard gate implementation and its configuration.
    pub hard_gates: ArtifactIdentity,
    /// The engine and vehicle action-port identity for scenario execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_runtime: Option<ArtifactIdentity>,
    /// The simulator build and adapter configuration.
    pub simulator: ArtifactIdentity,
    /// The selected simulator airframe artifact.
    pub airframe: ArtifactIdentity,
    /// The vehicle controller build and adapter configuration.
    pub vehicle: ArtifactIdentity,
    /// The candidate-transition validator and its exact configuration.
    pub transition_validator: ArtifactIdentity,
    /// The exact vehicle adjacency-policy digest.
    pub adjacency_policy_digest: Digest,
}

impl RuntimeIdentities {
    /// Returns the evaluator identities this session is frozen to.
    #[must_use]
    pub fn evaluator_identities(&self) -> EvaluatorIdentities {
        EvaluatorIdentities {
            metric: self.metric.clone(),
            hard_gates: self.hard_gates.clone(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), TuneError> {
        self.evaluator_identities().validate()?;
        for identity in [
            &self.harness_build,
            &self.strategy,
            &self.metric,
            &self.hard_gates,
            &self.simulator,
            &self.airframe,
            &self.vehicle,
            &self.transition_validator,
        ] {
            identity.validate()?;
        }
        let scenario_runtime =
            self.scenario_runtime
                .as_ref()
                .ok_or_else(|| TuneError::InvalidIdentity {
                    detail: "the scenario runtime uses the prior identity domain".to_owned(),
                })?;
        scenario_runtime.validate()?;
        if scenario_runtime.id != SCENARIO_RUNTIME_ID_V2 {
            return Err(TuneError::InvalidIdentity {
                detail: "the scenario runtime uses the prior identity domain".to_owned(),
            });
        }
        if self.adjacency_policy_digest.is_zero() {
            return Err(TuneError::InvalidIdentity {
                detail: "the vehicle adjacency-policy digest is zero".to_owned(),
            });
        }
        Ok(())
    }
}

/// Returns the production-source identity of the shared mission engine.
#[must_use]
pub fn scenario_engine_identity() -> ArtifactIdentity {
    ArtifactIdentity {
        id: "pilotage-scenario-engine-source-v2".to_owned(),
        digest: digest_bytes(env!("FLIGHT_TUNE_SCENARIO_ENGINE_ID").as_bytes()),
    }
}

/// Composes the shared engine and vehicle action-port runtime identity.
///
/// # Errors
///
/// Returns [`TuneError`] when the vehicle action-port identity is invalid.
pub fn scenario_runtime_identity(
    vehicle_action_port: &ArtifactIdentity,
) -> Result<ArtifactIdentity, TuneError> {
    vehicle_action_port.validate()?;
    let engine = scenario_engine_identity();
    let mut hasher = Sha256::new();
    hasher.update(b"pilotage-scenario-runtime-v2\0");
    hasher.update(engine.digest.as_bytes());
    hasher.update((vehicle_action_port.id.len() as u64).to_le_bytes());
    hasher.update(vehicle_action_port.id.as_bytes());
    hasher.update(vehicle_action_port.digest.as_bytes());
    ArtifactIdentity::new(
        SCENARIO_RUNTIME_ID_V2,
        Digest::from_bytes(hasher.finalize().into()),
    )
}

pub(crate) fn harness_build_identity() -> ArtifactIdentity {
    ArtifactIdentity {
        id: "flight-tune-build".to_owned(),
        digest: digest_bytes(env!("FLIGHT_TUNE_BUILD_ID").as_bytes()),
    }
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest::from_bytes(hasher.finalize().into())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn vehicle_action_port_change_resets_final_runtime_identity() {
        let first = ArtifactIdentity::from_text("aviate-action-port", "first").expect("identity");
        let second = ArtifactIdentity::from_text("aviate-action-port", "second").expect("identity");

        assert_ne!(
            scenario_runtime_identity(&first).expect("first runtime"),
            scenario_runtime_identity(&second).expect("second runtime")
        );
    }

    #[test]
    fn prior_runtime_identity_remains_readable_but_is_not_admitted() {
        let runtimes = runtimes();
        let mut value = serde_json::to_value(&runtimes).expect("encode runtimes");
        value["scenario_runtime"]["id"] =
            Value::String("flight-tune-aviate-scenario-runtime".to_owned());
        let prior: RuntimeIdentities =
            serde_json::from_value(value).expect("read prior runtime evidence");

        assert_eq!(
            prior
                .scenario_runtime
                .as_ref()
                .map(|identity| identity.id.as_str()),
            Some("flight-tune-aviate-scenario-runtime")
        );
        assert!(prior.validate().is_err());
        let encoded = serde_json::to_value(prior).expect("encode prior evidence");
        assert_eq!(
            encoded["scenario_runtime"]["id"],
            Value::String("flight-tune-aviate-scenario-runtime".to_owned())
        );
    }

    fn runtimes() -> RuntimeIdentities {
        RuntimeIdentities {
            harness_build: harness_build_identity(),
            strategy: identity("strategy", 1),
            metric: identity("metric", 2),
            hard_gates: identity("gates", 3),
            scenario_runtime: Some(identity(SCENARIO_RUNTIME_ID_V2, 4)),
            simulator: identity("simulator", 5),
            airframe: identity("airframe", 6),
            vehicle: identity("vehicle", 7),
            transition_validator: identity("transition-validator", 8),
            adjacency_policy_digest: Digest::from_bytes([9; 32]),
        }
    }

    fn identity(id: &str, byte: u8) -> ArtifactIdentity {
        ArtifactIdentity::new(id, Digest::from_bytes([byte; 32])).expect("identity")
    }
}

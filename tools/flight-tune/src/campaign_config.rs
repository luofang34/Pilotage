use serde::{Deserialize, Serialize};

use crate::{SearchStage, TuneError};

/// The supported neutral campaign configuration schema.
///
/// The document embeds a complete search stage, so a changed stage shape is a
/// changed configuration shape. An unbumped version would let two different
/// shapes claim one schema, and a document written against the older one would
/// fail to decode with no version to explain why.
pub const CAMPAIGN_CONFIG_SCHEMA_VERSION: u16 = 2;

/// A simulator-neutral tuning campaign document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignConfig {
    /// The campaign configuration schema.
    pub schema_version: u16,
    /// The stable campaign identifier.
    pub campaign_id: String,
    /// The fixed proposal and run seed.
    pub fixed_seed: u64,
    /// The ordered search stages.
    pub stages: Vec<SearchStage>,
}

/// Typed simulator and vehicle adapter documents for one campaign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignAdapterDocuments<S, V> {
    /// The simulator adapter document.
    pub simulator: S,
    /// The vehicle adapter document.
    pub vehicle: V,
}

impl CampaignConfig {
    /// Validates the neutral campaign document.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the schema, identifier, or stages are invalid.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != CAMPAIGN_CONFIG_SCHEMA_VERSION {
            return Err(TuneError::InvalidStage {
                detail: "the campaign configuration schema is not supported".to_owned(),
            });
        }
        if self.campaign_id.trim().is_empty() || self.campaign_id.len() > 128 {
            return Err(TuneError::InvalidStage {
                detail: "the campaign identifier is empty or too long".to_owned(),
            });
        }
        if self.stages.is_empty() {
            return Err(TuneError::InvalidStage {
                detail: "the campaign has no search stage".to_owned(),
            });
        }
        for stage in &self.stages {
            stage.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        Digest, MissionReference, PROMOTION_POLICY_SCHEMA_VERSION, ParameterBounds,
        PromotionPolicy, PromotionSeedPolicy, QualificationPolicy,
    };

    #[test]
    fn neutral_document_has_no_required_adapter_block() {
        let value = serde_json::json!({
            "schema_version": CAMPAIGN_CONFIG_SCHEMA_VERSION,
            "campaign_id": "neutral-campaign",
            "fixed_seed": 7,
            "stages": [stage()],
        });
        let document: CampaignConfig = serde_json::from_value(value).expect("decode campaign");

        document.validate().expect("validate neutral campaign");
    }

    fn limits() -> BTreeMap<String, f64> {
        BTreeMap::from([("response".to_owned(), 1.0)])
    }

    fn stage() -> SearchStage {
        SearchStage {
            execution_retry: crate::ExecutionRetryPolicy::none(),
            id: "neutral-stage".to_owned(),
            allowlist: BTreeMap::from([(
                "gain".to_owned(),
                ParameterBounds {
                    minimum: 0.0,
                    maximum: 1.0,
                },
            )]),
            fixed_parameters: BTreeMap::new(),
            required_hard_gates: vec![
                crate::MANDATORY_CRASH_GATE_ID.to_owned(),
                "envelope".to_owned(),
            ],
            training_scenarios: vec![scenario("training", 1)],
            training_suites: vec![crate::TrainingSuite {
                schema_version: crate::TRAINING_SUITE_SCHEMA_VERSION,
                id: "neutral-suite".to_owned(),
                primary_scenarios: vec![scenario("training", 1)],
                guard_scenarios: Vec::new(),
                guard_regression_limits: BTreeMap::new(),
                repetitions: 2,
            }],
            search_groups: vec![crate::SearchGroup {
                id: "neutral-group".to_owned(),
                kind: crate::SearchGroupKind::Controller,
                parameters: std::collections::BTreeSet::from(["gain".to_owned()]),
                suite_id: "neutral-suite".to_owned(),
            }],
            promotion_scenarios: vec![scenario("promotion", 2)],
            final_qualification_scenarios: vec![scenario("final", 3)],
            repetitions: 2,
            promotion: PromotionPolicy {
                schema_version: PROMOTION_POLICY_SCHEMA_VERSION,
                seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
                minimum_loss_improvement: 0.0,
                minimum_relative_loss_improvement: 0.0,
                maximum_control_effort_increase: 0.0,
                objectives: std::collections::BTreeSet::from(["response".to_owned()]),
            },
            qualification: QualificationPolicy {
                maximum_loss_confidence_upper: 1.0,
                maximum_p95_loss: 1.0,
                maximum_mean_control_effort: 1.0,
                objectives: std::collections::BTreeSet::from(["response".to_owned()]),
            },
            response_targets: crate::model::response_target::fixture::covering(&[
                (&[scenario("promotion", 2)], &limits()),
                (&[scenario("final", 3)], &limits()),
            ]),
        }
    }

    fn scenario(id: &str, byte: u8) -> MissionReference {
        MissionReference {
            revision_id: id.to_owned(),
            schema_version: flight_tune::MISSION_SCHEMA_VERSION,
            content_digest: Digest::from_bytes([byte; 32]),
            max_samples: 1,
            sample_timeout_ns: 1_000_000,
        }
    }
}

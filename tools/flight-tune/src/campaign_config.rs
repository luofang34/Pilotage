use serde::{Deserialize, Serialize};

use crate::{SearchStage, TuneError};

/// The supported neutral campaign configuration schema.
pub const CAMPAIGN_CONFIG_SCHEMA_VERSION: u16 = 1;

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
        Digest, PROMOTION_POLICY_SCHEMA_VERSION, ParameterBounds, PromotionPolicy,
        PromotionSeedPolicy, QualificationPolicy, ScenarioRef,
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

    fn stage() -> SearchStage {
        SearchStage {
            id: "neutral-stage".to_owned(),
            allowlist: BTreeMap::from([(
                "gain".to_owned(),
                ParameterBounds {
                    minimum: 0.0,
                    maximum: 1.0,
                },
            )]),
            fixed_parameters: BTreeMap::new(),
            required_hard_gates: vec!["envelope".to_owned()],
            training_scenarios: vec![scenario("training", 1)],
            promotion_scenarios: vec![scenario("promotion", 2)],
            final_qualification_scenarios: vec![scenario("final", 3)],
            repetitions: 2,
            promotion: PromotionPolicy {
                schema_version: PROMOTION_POLICY_SCHEMA_VERSION,
                seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
                minimum_loss_improvement: 0.0,
                minimum_relative_loss_improvement: 0.0,
                maximum_control_effort_increase: 0.0,
                objective_regression_upper_95: BTreeMap::from([("response".to_owned(), 1.0)]),
            },
            qualification: QualificationPolicy {
                maximum_loss_confidence_upper: 1.0,
                maximum_p95_loss: 1.0,
                maximum_mean_control_effort: 1.0,
                objective_maxima: BTreeMap::from([("response".to_owned(), 1.0)]),
            },
        }
    }

    fn scenario(id: &str, byte: u8) -> ScenarioRef {
        ScenarioRef {
            id: id.to_owned(),
            digest: Digest::from_bytes([byte; 32]),
            max_samples: 1,
            sample_timeout_ms: 1,
        }
    }
}

use std::collections::{BTreeSet, HashSet};

use flight_tune::{ArtifactIdentity, Digest, PromotionSeedPolicy, ScenarioRef, SearchStage};

use crate::{FeedbackError, error::invalid};

const MAX_NAME_BYTES: usize = 128;
const MAX_PARAMETERS: usize = 128;
const MAX_SCENARIOS_PER_SET: usize = 64;
const MAX_PROMOTION_OBJECTIVES: usize = 64;
const PROMOTION_POLICY_SCHEMA_VERSION: u16 = 1;

pub(super) fn verify(stage: &SearchStage) -> Result<(), FeedbackError> {
    verify_name(&stage.id, "stage")?;
    verify_counts(stage)?;
    verify_parameters(stage)?;
    verify_scenarios(stage)?;
    verify_promotion(stage)?;
    verify_qualification(stage)
}

pub(super) fn verify_artifact(
    artifact: &ArtifactIdentity,
    name: &'static str,
) -> Result<(), FeedbackError> {
    if artifact.id.trim().is_empty() || artifact.id.len() > 256 || artifact.digest.is_zero() {
        return Err(invalid(format!("the {name} identity is not valid")));
    }
    Ok(())
}

fn verify_counts(stage: &SearchStage) -> Result<(), FeedbackError> {
    if stage.allowlist.is_empty()
        || stage.allowlist.len() > MAX_PARAMETERS
        || !(2..=32).contains(&stage.repetitions)
        || stage.required_hard_gates.is_empty()
        || stage.required_hard_gates.len() > 32
    {
        return Err(invalid("the search stage count is not valid"));
    }
    let mut gates = BTreeSet::new();
    for gate in &stage.required_hard_gates {
        verify_name(gate, "hard gate")?;
        if !gates.insert(gate) {
            return Err(invalid("a required hard gate is repeated"));
        }
    }
    Ok(())
}

fn verify_parameters(stage: &SearchStage) -> Result<(), FeedbackError> {
    for (name, bounds) in &stage.allowlist {
        verify_name(name, "parameter")?;
        if !bounds.minimum.is_finite()
            || !bounds.maximum.is_finite()
            || bounds.minimum >= bounds.maximum
            || stage.fixed_parameters.contains_key(name)
        {
            return Err(invalid(format!("the bounds for {name} are not valid")));
        }
    }
    for (name, value) in &stage.fixed_parameters {
        verify_name(name, "fixed parameter")?;
        if !value.is_finite() {
            return Err(invalid(format!("fixed parameter {name} is not finite")));
        }
    }
    Ok(())
}

fn verify_scenarios(stage: &SearchStage) -> Result<(), FeedbackError> {
    let mut ids = BTreeSet::new();
    let mut digests = HashSet::new();
    for scenarios in [
        &stage.training_scenarios,
        &stage.promotion_scenarios,
        &stage.final_qualification_scenarios,
    ] {
        if scenarios.is_empty() || scenarios.len() > MAX_SCENARIOS_PER_SET {
            return Err(invalid("a search stage scenario set size is not valid"));
        }
        for scenario in scenarios {
            verify_scenario(scenario, &mut ids, &mut digests)?;
        }
    }
    Ok(())
}

fn verify_scenario(
    scenario: &ScenarioRef,
    ids: &mut BTreeSet<String>,
    digests: &mut HashSet<Digest>,
) -> Result<(), FeedbackError> {
    verify_name(&scenario.id, "scenario")?;
    if !ids.insert(scenario.id.clone())
        || !digests.insert(scenario.digest)
        || scenario.digest.is_zero()
        || scenario.max_samples == 0
        || scenario.sample_timeout_ms == 0
        || scenario.sample_timeout_ms > 60_000
    {
        return Err(invalid(format!(
            "scenario {} is repeated or has invalid limits",
            scenario.id
        )));
    }
    Ok(())
}

fn verify_promotion(stage: &SearchStage) -> Result<(), FeedbackError> {
    let policy = &stage.promotion;
    if policy.schema_version != PROMOTION_POLICY_SCHEMA_VERSION
        || policy.seed_policy != PromotionSeedPolicy::PairedScenarioDigestV1
        || !nonnegative(policy.minimum_loss_improvement)
        || !policy.minimum_relative_loss_improvement.is_finite()
        || !(0.0..=1.0).contains(&policy.minimum_relative_loss_improvement)
        || !policy.maximum_control_effort_increase.is_finite()
        || !(0.0..=1.0).contains(&policy.maximum_control_effort_increase)
        || policy.objective_regression_upper_95.is_empty()
        || policy.objective_regression_upper_95.len() > MAX_PROMOTION_OBJECTIVES
    {
        return Err(invalid("the promotion policy is not valid"));
    }
    for (name, maximum) in &policy.objective_regression_upper_95 {
        if name.is_empty()
            || name.len() > MAX_NAME_BYTES
            || name.chars().any(char::is_whitespace)
            || !nonnegative(*maximum)
        {
            return Err(invalid("a promotion objective limit is not valid"));
        }
    }
    Ok(())
}

fn verify_qualification(stage: &SearchStage) -> Result<(), FeedbackError> {
    let policy = &stage.qualification;
    if !nonnegative(policy.maximum_loss_confidence_upper)
        || !nonnegative(policy.maximum_p95_loss)
        || !policy.maximum_mean_control_effort.is_finite()
        || !(0.0..=1.0).contains(&policy.maximum_mean_control_effort)
    {
        return Err(invalid("the final qualification policy is not valid"));
    }
    for (name, maximum) in &policy.objective_maxima {
        verify_name(name, "qualification objective")?;
        if !nonnegative(*maximum) {
            return Err(invalid("a qualification objective limit is not valid"));
        }
    }
    Ok(())
}

fn verify_name(value: &str, kind: &'static str) -> Result<(), FeedbackError> {
    if value.trim().is_empty() || value.len() > MAX_NAME_BYTES {
        return Err(invalid(format!("the {kind} name is not valid")));
    }
    Ok(())
}

const fn nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

use flight_tune::{
    AttemptRole, Digest, MissionReference, RunTerminalReceipt, ScenarioSet, SearchStage,
};
use serde::Serialize;

use crate::{FeedbackError, digest, error::invalid};

use super::training_suite;

const RUN_CONTEXT_DOMAIN: &[u8] = b"flight-tune:run-execution-context:v4\0";

#[derive(Clone)]
pub(super) struct ExpectedRun {
    pub(super) role: AttemptRole,
    pub(super) candidate: Digest,
    pub(super) trial_id: u64,
    pub(super) scenario_set: ScenarioSet,
    pub(super) scenario: MissionReference,
    pub(super) repetition: u32,
    pub(super) seed: u64,
    pub(super) session_digest: Digest,
    pub(super) retry_index: u32,
}

/// The identity of the suite that one training run plan uses.
#[derive(Serialize)]
struct SuiteAnchor {
    index: u16,
    id: String,
    digest: Digest,
}

#[derive(Serialize)]
struct RunPlanDocument<'a> {
    role: AttemptRole,
    candidate: Digest,
    scenario_set: ScenarioSet,
    scenarios: &'a [MissionReference],
    repetitions: u32,
    fixed_seed: u64,
    training_suite: Option<&'a SuiteAnchor>,
}

/// The ordered run plan one role takes from one frozen stage.
struct AttemptPlan {
    scenarios: Vec<MissionReference>,
    repetitions: u32,
    suite: Option<SuiteAnchor>,
}

fn attempt_plan(stage: &SearchStage, role: AttemptRole) -> Result<AttemptPlan, FeedbackError> {
    let set = scenario_set(role);
    let Some(index) = training_suite_index(role) else {
        return Ok(AttemptPlan {
            scenarios: scenarios(stage, set).to_vec(),
            repetitions: stage.repetitions,
            suite: None,
        });
    };
    let suite = training_suite::suite_at(stage, index)?;
    Ok(AttemptPlan {
        scenarios: training_suite::ordered_scenarios(suite),
        repetitions: suite.repetitions,
        suite: Some(SuiteAnchor {
            index,
            id: suite.id.clone(),
            digest: training_suite::suite_digest(suite)?,
        }),
    })
}

const fn training_suite_index(role: AttemptRole) -> Option<u16> {
    match role {
        AttemptRole::TrainingBaseline { suite_index }
        | AttemptRole::TrainingChallenger { suite_index, .. } => Some(suite_index),
        AttemptRole::PromotionBaseline
        | AttemptRole::PromotionFrozen
        | AttemptRole::FinalQualification => None,
    }
}

pub(super) fn digest_for(
    stage: &SearchStage,
    role: AttemptRole,
    candidate: Digest,
    fixed_seed: u64,
) -> Result<Digest, FeedbackError> {
    let plan = attempt_plan(stage, role)?;
    digest::document(
        "run plan",
        &RunPlanDocument {
            role,
            candidate,
            scenario_set: scenario_set(role),
            scenarios: &plan.scenarios,
            repetitions: plan.repetitions,
            fixed_seed,
            training_suite: plan.suite.as_ref(),
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn expected_runs(
    stage: &SearchStage,
    role: AttemptRole,
    candidate: Digest,
    trial_id: u64,
    fixed_seed: u64,
    session_digest: Digest,
    retry_index: u32,
) -> Result<Vec<ExpectedRun>, FeedbackError> {
    let scenario_set = scenario_set(role);
    let plan = attempt_plan(stage, role)?;
    let capacity = plan
        .scenarios
        .len()
        .saturating_mul(plan.repetitions as usize);
    let mut expected = Vec::with_capacity(capacity);
    for scenario in &plan.scenarios {
        for repetition in 0..plan.repetitions {
            expected.push(ExpectedRun {
                role,
                candidate,
                trial_id,
                scenario_set,
                scenario: scenario.clone(),
                repetition,
                seed: derive_seed(fixed_seed, scenario_set, scenario, repetition),
                session_digest,
                retry_index,
            });
        }
    }
    Ok(expected)
}

pub(super) fn verify_receipt_context(
    receipt: &RunTerminalReceipt,
    expected: &ExpectedRun,
) -> Result<(), FeedbackError> {
    let context = receipt.context();
    let intent_digest = digest::domain("run execution context", RUN_CONTEXT_DOMAIN, context)?;
    if context.tuning_session_digest() != expected.session_digest
        || context.trial_id() != expected.trial_id
        || context.role() != expected.role
        || context.candidate_digest() != expected.candidate
        || context.transition_authorization().is_some()
        || context.scenario_set() != expected.scenario_set
        || context.mission_revision_id() != expected.scenario.revision_id
        || context.mission_content_digest() != expected.scenario.content_digest
        || context.repetition() != expected.repetition
        || context.seed() != expected.seed
        || context.retry_index() != expected.retry_index
        || receipt.intent().run_intent_digest() != intent_digest
    {
        return Err(invalid(
            "a terminal receipt does not match the expected run plan",
        ));
    }
    Ok(())
}

pub(super) const fn scenario_set(role: AttemptRole) -> ScenarioSet {
    match role {
        AttemptRole::TrainingBaseline { .. } | AttemptRole::TrainingChallenger { .. } => {
            ScenarioSet::Training
        }
        AttemptRole::PromotionBaseline | AttemptRole::PromotionFrozen => ScenarioSet::Promotion,
        AttemptRole::FinalQualification => ScenarioSet::FinalQualification,
    }
}

fn scenarios(stage: &SearchStage, set: ScenarioSet) -> &[MissionReference] {
    match set {
        ScenarioSet::Training => &stage.training_scenarios,
        ScenarioSet::Promotion => &stage.promotion_scenarios,
        ScenarioSet::FinalQualification => &stage.final_qualification_scenarios,
    }
}

fn derive_seed(
    fixed_seed: u64,
    set: ScenarioSet,
    scenario: &MissionReference,
    repetition: u32,
) -> u64 {
    let partition = match set {
        ScenarioSet::Training => 0x243f_6a88_85a3_08d3,
        ScenarioSet::Promotion => 0x1319_8a2e_0370_7344,
        ScenarioSet::FinalQualification => 0xa409_3822_299f_31d0,
    };
    let bytes = scenario.content_digest.as_bytes();
    let key = digest_word(bytes, 0)
        ^ digest_word(bytes, 8).rotate_left(13)
        ^ digest_word(bytes, 16).rotate_left(29)
        ^ digest_word(bytes, 24).rotate_left(47);
    split_mix(fixed_seed ^ partition ^ key ^ u64::from(repetition))
}

fn digest_word(bytes: &[u8; 32], start: usize) -> u64 {
    u64::from_le_bytes([
        bytes[start],
        bytes[start.wrapping_add(1)],
        bytes[start.wrapping_add(2)],
        bytes[start.wrapping_add(3)],
        bytes[start.wrapping_add(4)],
        bytes[start.wrapping_add(5)],
        bytes[start.wrapping_add(6)],
        bytes[start.wrapping_add(7)],
    ])
}

fn split_mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use flight_tune::{Digest, MissionReference, ScenarioSet};

    use super::derive_seed;

    #[test]
    fn promotion_seed_v1_outputs_are_fixed() {
        let scenario = MissionReference {
            revision_id: "promotion-calm".to_owned(),
            schema_version: flight_tune::MISSION_SCHEMA_VERSION,
            content_digest: Digest::from_bytes([12; 32]),
            max_samples: 100,
            sample_timeout_ns: 20_000_000,
        };
        let seeds = (0..3)
            .map(|repetition| derive_seed(23, ScenarioSet::Promotion, &scenario, repetition))
            .collect::<Vec<_>>();
        assert_eq!(
            seeds,
            vec![
                0xca8d_f280_ece5_22d8,
                0xacf4_320a_124a_f300,
                0xeb00_2810_ace4_6163,
            ]
        );
    }
}

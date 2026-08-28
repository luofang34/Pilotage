use flight_tune::{AttemptRole, Digest, RunTerminalReceipt, ScenarioRef, ScenarioSet, SearchStage};
use serde::Serialize;

use crate::{FeedbackError, digest, error::invalid};

const RUN_CONTEXT_DOMAIN: &[u8] = b"flight-tune:run-execution-context:v1\0";

#[derive(Clone, Copy)]
pub(super) struct ExpectedRun<'a> {
    pub(super) role: AttemptRole,
    pub(super) candidate: Digest,
    pub(super) trial_id: u64,
    pub(super) scenario_set: ScenarioSet,
    pub(super) scenario: &'a ScenarioRef,
    pub(super) repetition: u32,
    pub(super) seed: u64,
    pub(super) session_digest: Digest,
}

#[derive(Serialize)]
struct RunPlanDocument<'a> {
    role: AttemptRole,
    candidate: Digest,
    scenario_set: ScenarioSet,
    scenarios: &'a [ScenarioRef],
    repetitions: u32,
    fixed_seed: u64,
}

pub(super) fn digest_for(
    stage: &SearchStage,
    role: AttemptRole,
    candidate: Digest,
    fixed_seed: u64,
) -> Result<Digest, FeedbackError> {
    let scenario_set = scenario_set(role);
    digest::document(
        "run plan",
        &RunPlanDocument {
            role,
            candidate,
            scenario_set,
            scenarios: scenarios(stage, scenario_set),
            repetitions: stage.repetitions,
            fixed_seed,
        },
    )
}

pub(super) fn expected_runs(
    stage: &SearchStage,
    role: AttemptRole,
    candidate: Digest,
    trial_id: u64,
    fixed_seed: u64,
    session_digest: Digest,
) -> Vec<ExpectedRun<'_>> {
    let scenario_set = scenario_set(role);
    let capacity = scenarios(stage, scenario_set)
        .len()
        .saturating_mul(stage.repetitions as usize);
    let mut expected = Vec::with_capacity(capacity);
    for scenario in scenarios(stage, scenario_set) {
        for repetition in 0..stage.repetitions {
            expected.push(ExpectedRun {
                role,
                candidate,
                trial_id,
                scenario_set,
                scenario,
                repetition,
                seed: derive_seed(fixed_seed, scenario_set, scenario, repetition),
                session_digest,
            });
        }
    }
    expected
}

pub(super) fn verify_receipt_context(
    receipt: &RunTerminalReceipt,
    expected: ExpectedRun<'_>,
) -> Result<(), FeedbackError> {
    let context = receipt.context();
    let intent_digest = digest::domain("run execution context", RUN_CONTEXT_DOMAIN, context)?;
    if context.tuning_session_digest() != expected.session_digest
        || context.trial_id() != expected.trial_id
        || context.role() != expected.role
        || context.candidate_digest() != expected.candidate
        || context.transition_authorization().is_some()
        || context.scenario_set() != expected.scenario_set
        || context.scenario_id() != expected.scenario.id
        || context.scenario_digest() != expected.scenario.digest
        || context.repetition() != expected.repetition
        || context.seed() != expected.seed
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
        AttemptRole::TrainingBaseline | AttemptRole::TrainingChallenger { .. } => {
            ScenarioSet::Training
        }
        AttemptRole::PromotionBaseline | AttemptRole::PromotionFrozen => ScenarioSet::Promotion,
        AttemptRole::FinalQualification => ScenarioSet::FinalQualification,
    }
}

fn scenarios(stage: &SearchStage, set: ScenarioSet) -> &[ScenarioRef] {
    match set {
        ScenarioSet::Training => &stage.training_scenarios,
        ScenarioSet::Promotion => &stage.promotion_scenarios,
        ScenarioSet::FinalQualification => &stage.final_qualification_scenarios,
    }
}

fn derive_seed(fixed_seed: u64, set: ScenarioSet, scenario: &ScenarioRef, repetition: u32) -> u64 {
    let partition = match set {
        ScenarioSet::Training => 0x243f_6a88_85a3_08d3,
        ScenarioSet::Promotion => 0x1319_8a2e_0370_7344,
        ScenarioSet::FinalQualification => 0xa409_3822_299f_31d0,
    };
    let bytes = scenario.digest.as_bytes();
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
    use flight_tune::{Digest, ScenarioRef, ScenarioSet};

    use super::derive_seed;

    #[test]
    fn promotion_seed_v1_outputs_are_fixed() {
        let scenario = ScenarioRef {
            id: "promotion-calm".to_owned(),
            digest: Digest::from_bytes([12; 32]),
            max_samples: 100,
            sample_timeout_ms: 20,
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

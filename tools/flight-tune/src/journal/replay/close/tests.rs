#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use crate::journal::{CampaignPhase, PromotionClosure, SessionIdentity};
use crate::{
    ArtifactIdentity, AttemptRole, CandidateLineage, Digest, PromotionDecision, PromotionPolicy,
    PromotionSeedPolicy, PromotionSelection, QualificationPolicy, RuntimeIdentities, SearchStage,
};

use super::promotion;

#[test]
fn replay_rejects_a_promotion_closure_without_an_initial_proof() {
    let stage = stage();
    let initial = digest(1);
    let mut state = super::super::initial_state(initial);
    state.phase = CampaignPhase::Frozen;
    state.frozen_candidate = Some(digest(2));
    let closure = PromotionClosure::new(
        crate::promotion_policy_digest(&stage.promotion).expect("policy digest"),
        None,
        None,
        None,
        PromotionSelection {
            decision: PromotionDecision::Indeterminate {
                reason: "forged missing baseline".to_owned(),
            },
            selected_candidate: None,
        },
    )
    .expect("syntactically valid closure");

    assert!(promotion(&mut state, &closure, &stage, &session(initial)).is_err());
    assert_eq!(state.phase, CampaignPhase::Frozen);
    assert!(state.promotion_closure.is_none());
}

#[test]
fn prospective_replay_rejects_final_attempts_without_promotion_authority() {
    let stage = stage();
    let initial = digest(1);
    for decision in [
        PromotionDecision::RejectedHardGate {
            gate_id: "test.gate".to_owned(),
        },
        PromotionDecision::Indeterminate {
            reason: "test quarantine".to_owned(),
        },
    ] {
        let mut state = super::super::initial_state(initial);
        state.phase = CampaignPhase::PromotionClosed;
        state.promotion_decision = Some(decision.clone());
        state.promotion_closure = Some(
            PromotionClosure::new(
                crate::promotion_policy_digest(&stage.promotion).expect("policy digest"),
                Some((digest(16), digest(17))),
                None,
                None,
                PromotionSelection {
                    decision,
                    selected_candidate: None,
                },
            )
            .expect("rejected promotion closure"),
        );
        let role = AttemptRole::FinalQualification;
        let plan = role
            .plan_digest(&stage, initial, session(initial).fixed_seed)
            .expect("final plan");

        assert!(
            super::super::attempt::prepare(
                &mut state, 0, role, initial, plan, None, &stage, initial, 6,
            )
            .is_err()
        );
        assert!(state.pending.is_none());
        assert_eq!(state.next_trial_id, 0);
    }
}

fn stage() -> SearchStage {
    SearchStage {
        id: String::new(),
        allowlist: BTreeMap::new(),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: Vec::new(),
        training_scenarios: Vec::new(),
        promotion_scenarios: Vec::new(),
        final_qualification_scenarios: Vec::new(),
        repetitions: 0,
        promotion: PromotionPolicy {
            schema_version: crate::PROMOTION_POLICY_SCHEMA_VERSION,
            seed_policy: PromotionSeedPolicy::PairedScenarioDigestV1,
            minimum_loss_improvement: 0.1,
            minimum_relative_loss_improvement: 0.1,
            maximum_control_effort_increase: 0.1,
            objective_regression_upper_95: BTreeMap::from([("tracking".to_owned(), 0.1)]),
        },
        qualification: QualificationPolicy {
            maximum_loss_confidence_upper: 1.0,
            maximum_p95_loss: 1.0,
            maximum_mean_control_effort: 1.0,
            objective_maxima: BTreeMap::new(),
        },
    }
}

fn session(initial: Digest) -> SessionIdentity {
    SessionIdentity {
        stage_digest: digest(3),
        initial_candidate_digest: initial,
        candidate_lineage: CandidateLineage {
            schema: "test".to_owned(),
            base_preset_digest: digest(4),
            plant_digest: digest(5),
        },
        fixed_seed: 6,
        runtimes: RuntimeIdentities {
            harness_build: identity("harness", 7),
            strategy: identity("strategy", 8),
            metric: identity("metric", 9),
            hard_gates: identity("gates", 10),
            scenario_runtime: Some(identity("pilotage-scenario-runtime-v2", 16)),
            simulator: identity("simulator", 11),
            airframe: identity("airframe", 12),
            vehicle: identity("vehicle", 13),
            transition_validator: identity("validator", 14),
            adjacency_policy_digest: digest(15),
        },
    }
}

fn identity(id: &str, value: u8) -> ArtifactIdentity {
    ArtifactIdentity {
        id: id.to_owned(),
        digest: digest(value),
    }
}

fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}

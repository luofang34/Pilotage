use serde_json::Value;

use super::super::calculate;
use super::{
    MetricPoint, evidence, expected_pairs, fixed_digest, plan, receipt_for_context, stage,
};
use crate::{
    AttemptRole, MissionReference, RunExecutionContext, RunRecord, RunTerminalClass,
    RunTerminalReceipt, ScenarioSet,
};

#[derive(Clone, Copy)]
enum IdentityChange {
    Session,
    Trial,
    Role,
    Candidate,
    ScenarioId,
    ScenarioDigest,
    Repetition,
    Seed,
}

#[test]
fn missing_extra_repeated_and_swapped_receipts_fail() {
    let stage = stage();
    let original = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());

    let mut changed = original.baseline.clone();
    changed.remove(1);
    assert!(calculate(&stage, plan(), &changed, &original.frozen).is_err());

    let mut changed = original.baseline.clone();
    changed.push(changed[0].clone());
    assert!(calculate(&stage, plan(), &changed, &original.frozen).is_err());

    let mut changed = original.baseline.clone();
    changed[1] = changed[0].clone();
    assert!(calculate(&stage, plan(), &changed, &original.frozen).is_err());

    let mut changed = original.baseline.clone();
    changed.swap(0, 1);
    assert!(calculate(&stage, plan(), &changed, &original.frozen).is_err());

    let mut changed = original.baseline;
    changed[0] = original.frozen[0].clone();
    assert!(calculate(&stage, plan(), &changed, &original.frozen).is_err());
}

#[test]
fn every_context_key_and_attempt_identity_must_match() {
    let changes = [
        IdentityChange::Session,
        IdentityChange::Trial,
        IdentityChange::Role,
        IdentityChange::Candidate,
        IdentityChange::ScenarioId,
        IdentityChange::ScenarioDigest,
        IdentityChange::Repetition,
        IdentityChange::Seed,
    ];
    for change in changes {
        let stage = stage();
        let pairs = expected_pairs(&stage);
        let mut evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
        let context = changed_context(&pairs[0].baseline.context, change);
        evidence.baseline[0] = matching_receipt(&context, MetricPoint::baseline());
        assert!(calculate(&stage, plan(), &evidence.baseline, &evidence.frozen).is_err());
    }
}

#[test]
fn a_changed_run_intent_digest_fails_canonical_receipt_validation() {
    let stage = stage();
    let mut evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
    let mut document = serde_json::to_value(&evidence.baseline[0]).expect("encode receipt");
    document["run_intent_digest"] = serde_json::to_value(fixed_digest(98)).expect("encode digest");
    evidence.baseline[0] =
        serde_json::from_value(document).expect("decode changed receipt document");

    assert!(calculate(&stage, plan(), &evidence.baseline, &evidence.frozen).is_err());
}

#[test]
fn quarantine_receipts_cannot_supply_a_promotion_run() {
    let stage = stage();
    let mut evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
    let receipt = &evidence.baseline[0];
    let class = RunTerminalClass::evidence_failure(receipt.intent(), receipt.report())
        .expect("classify evidence failure");
    evidence.baseline[0] = RunTerminalReceipt::new(
        receipt.binding(),
        receipt.intent(),
        receipt.report(),
        class,
        fixed_digest(97),
    )
    .expect("create quarantine receipt");

    assert!(calculate(&stage, plan(), &evidence.baseline, &evidence.frozen).is_err());
}

#[test]
fn a_rechained_receipt_with_a_different_run_record_still_fails_expected_identity() {
    let stage = stage();
    let pairs = expected_pairs(&stage);
    let mut evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
    let context = changed_context(&pairs[0].baseline.context, IdentityChange::Seed);
    evidence.baseline[0] = matching_receipt(&context, MetricPoint::baseline());
    assert!(evidence.baseline[0].validate().is_ok());
    assert!(calculate(&stage, plan(), &evidence.baseline, &evidence.frozen).is_err());
}

#[test]
fn every_run_requires_the_exact_ordered_hard_gate_set() {
    let cases = [
        vec!["crash".to_owned()],
        vec!["crash".to_owned(), "finite".to_owned(), "extra".to_owned()],
        vec!["finite".to_owned(), "crash".to_owned()],
    ];
    for passed_hard_gates in cases {
        let mut stage = stage();
        stage.required_hard_gates = vec!["crash".to_owned(), "finite".to_owned()];
        let pairs = expected_pairs(&stage);
        let mut evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
        let context = &pairs[0].baseline.context;
        evidence.baseline[0] = receipt_for_context(
            context,
            RunRecord {
                scenario_set: ScenarioSet::Promotion,
                mission_revision_id: context.mission_revision_id().to_owned(),
                repetition: context.repetition(),
                seed: context.seed(),
                loss: MetricPoint::baseline().loss,
                control_effort: MetricPoint::baseline().effort,
                objectives: MetricPoint::baseline().objectives(),
                passed_hard_gates,
            },
        );

        assert!(calculate(&stage, plan(), &evidence.baseline, &evidence.frozen).is_err());
    }
}

fn changed_context(expected: &RunExecutionContext, change: IdentityChange) -> RunExecutionContext {
    let session = if matches!(change, IdentityChange::Session) {
        fixed_digest(70)
    } else {
        expected.tuning_session_digest()
    };
    let trial = if matches!(change, IdentityChange::Trial) {
        expected.trial_id().wrapping_add(1)
    } else {
        expected.trial_id()
    };
    let role = if matches!(change, IdentityChange::Role) {
        AttemptRole::PromotionFrozen
    } else {
        expected.role()
    };
    let candidate = if matches!(change, IdentityChange::Candidate) {
        fixed_digest(71)
    } else {
        expected.candidate_digest()
    };
    let scenario = changed_scenario(expected, change);
    let repetition = if matches!(change, IdentityChange::Repetition) {
        expected.repetition().wrapping_add(1)
    } else {
        expected.repetition()
    };
    let seed = if matches!(change, IdentityChange::Seed) {
        expected.seed().wrapping_add(1)
    } else {
        expected.seed()
    };
    RunExecutionContext::new(
        session,
        trial,
        role,
        candidate,
        None,
        ScenarioSet::Promotion,
        &scenario,
        repetition,
        seed,
    )
    .expect("create changed context")
}

fn changed_scenario(expected: &RunExecutionContext, change: IdentityChange) -> MissionReference {
    MissionReference {
        revision_id: if matches!(change, IdentityChange::ScenarioId) {
            "promotion-foreign".to_owned()
        } else {
            expected.mission_revision_id().to_owned()
        },
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: if matches!(change, IdentityChange::ScenarioDigest) {
            fixed_digest(72)
        } else {
            expected.mission_content_digest()
        },
        max_samples: 100,
        sample_timeout_ns: 20_000_000,
    }
}

fn matching_receipt(context: &RunExecutionContext, point: MetricPoint) -> RunTerminalReceipt {
    receipt_for_context(
        context,
        RunRecord {
            scenario_set: ScenarioSet::Promotion,
            mission_revision_id: context.mission_revision_id().to_owned(),
            repetition: context.repetition(),
            seed: context.seed(),
            loss: point.loss,
            control_effort: point.effort,
            objectives: point.objectives(),
            passed_hard_gates: vec!["crash".to_owned()],
        },
    )
}

#[test]
fn deserialization_keeps_unknown_receipt_fields_closed() {
    let stage = stage();
    let evidence = evidence(&stage, MetricPoint::baseline(), MetricPoint::passing());
    let mut document = serde_json::to_value(&evidence.baseline[0]).expect("encode receipt");
    document["foreign"] = Value::Bool(true);
    assert!(serde_json::from_value::<RunTerminalReceipt>(document).is_err());
}

use flight_tune::{
    ArtifactIdentity, AttemptRole, Digest, FinalQualificationOutcome, JournalEvent,
    JournalEvidenceSnapshot, MissionReference, PromotionDecision, RunBindingReceipt,
    RunExecutionContext, RunTerminalClass, RunTerminalIntent, RunTerminalReceipt,
    RunTerminalReport, RunTerminalSemanticOutcome, ScenarioSet,
};
use sha2::{Digest as ShaDigest, Sha256};

use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy, TestDirectory};

#[test]
fn every_proof_field_and_receipt_is_bound() {
    let exact = promotion_snapshot("proof-field-tamper");
    let frozen = exact
        .promotion_frozen
        .as_ref()
        .expect("frozen promotion proof");

    assert_proof_change_rejected(&exact, |proof| {
        proof.schema_version = proof.schema_version.wrapping_add(1);
    });
    assert_proof_change_rejected(&exact, |proof| {
        proof.trial_id = proof.trial_id.wrapping_add(1);
    });
    assert_proof_change_rejected(&exact, |proof| {
        proof.role = AttemptRole::PromotionFrozen;
    });
    assert_proof_change_rejected(&exact, |proof| {
        proof.candidate_digest = frozen.candidate_digest;
    });
    assert_proof_change_rejected(&exact, |proof| {
        proof.plan_digest = frozen.plan_digest;
    });
    assert_proof_change_rejected(&exact, |proof| {
        proof.evaluation = frozen.evaluation.clone();
    });
    for index in 0..exact.promotion_baseline.terminal_receipts.len() {
        assert_receipt_change_rejected(&exact, |proof| {
            proof.terminal_receipts[index] = frozen.terminal_receipts[index].clone();
        });
    }
    assert_receipt_change_rejected(&exact, |proof| {
        proof.terminal_receipts.swap(0, 1);
    });
    assert_receipt_change_rejected(&exact, |proof| {
        proof.terminal_receipts.pop();
    });
    assert_receipt_change_rejected(&exact, |proof| {
        let receipt = proof
            .terminal_receipts
            .last()
            .expect("baseline terminal receipt")
            .clone();
        proof.terminal_receipts.push(receipt);
    });
    assert_receipt_change_rejected(&exact, |proof| {
        proof.terminal_receipts[1] = proof.terminal_receipts[0].clone();
    });

    let mut changed = exact.clone();
    changed.promotion_baseline.evaluation_digest = digest(91);
    assert!(changed.validate().is_err());
    let mut changed = exact;
    changed.promotion_baseline.proof_digest = digest(92);
    assert!(changed.validate().is_err());
}

#[test]
fn both_promotion_proofs_reject_invalid_receipt_sequences() {
    let exact = promotion_snapshot("proof-receipt-shapes");
    let frozen = exact
        .promotion_frozen
        .as_ref()
        .expect("frozen promotion proof");

    assert_invalid_receipt_sequences(&exact.promotion_baseline, frozen);
    assert_invalid_receipt_sequences(frozen, &exact.promotion_baseline);
}

#[test]
fn every_promotion_closure_field_is_bound() {
    let exact = promotion_snapshot("closure-field-tamper");
    let initial = exact.head.entry.session.initial_candidate_digest;

    assert_closure_change_rejected(&exact, |closure| {
        closure.schema_version = closure.schema_version.wrapping_add(1);
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.policy_digest = digest(81);
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.baseline_evaluation_digest = Some(digest(82));
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.baseline_proof_digest = Some(digest(83));
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.frozen_evaluation_digest = Some(digest(84));
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.frozen_proof_digest = Some(digest(85));
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure
            .comparison
            .as_mut()
            .expect("promotion comparison")
            .baseline_mean_loss += 0.1;
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.comparison_digest = Some(digest(86));
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.decision = PromotionDecision::RejectedNoImprovement {};
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.decision_digest = digest(87);
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.selected_candidate = Some(initial);
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.selection_digest = digest(88);
    });
    assert_closure_change_rejected(&exact, |closure| {
        closure.closure_digest = digest(89);
    });

    let mut changed = exact.promotion_closure.clone();
    changed.frozen_evaluation_digest = None;
    changed.frozen_proof_digest = None;
    assert_ne!(
        changed
            .recompute_closure_digest()
            .expect("recompute closure without frozen proof"),
        exact.promotion_closure.closure_digest
    );
}

#[test]
fn sealed_snapshot_recomputes_candidate_and_outcome_from_final_proof() {
    let exact = sealed_snapshot("sealed-snapshot-authority");

    let mut changed_candidate = exact.clone();
    let JournalEvent::Sealed { candidate, .. } = &mut changed_candidate.head.entry.event else {
        panic!("sealed snapshot head");
    };
    *candidate = digest(93);
    refresh_head(&mut changed_candidate);
    assert!(changed_candidate.validate().is_err());

    let mut changed_outcome = exact;
    let forged = FinalQualificationOutcome::Indeterminate {
        reason: "forged final result".to_owned(),
    };
    let JournalEvent::Sealed { outcome, .. } = &mut changed_outcome.head.entry.event else {
        panic!("sealed snapshot head");
    };
    *outcome = forged.clone();
    changed_outcome.final_outcome = Some(forged);
    refresh_head(&mut changed_outcome);
    assert!(changed_outcome.validate().is_err());
}

#[test]
fn snapshot_rejects_rechained_run_context_and_vehicle_binding() {
    let exact = promotion_snapshot("snapshot-receipt-authority");

    let mut changed_adapter = exact.clone();
    let receipt = changed_adapter.promotion_baseline.terminal_receipts[0].clone();
    let adapter = ArtifactIdentity::new("foreign-vehicle", digest(94))
        .expect("create foreign vehicle identity");
    let binding = RunBindingReceipt::new(receipt.context(), receipt.report().plan(), adapter)
        .expect("bind foreign vehicle");
    changed_adapter.promotion_baseline.terminal_receipts[0] = RunTerminalReceipt::new(
        &binding,
        receipt.intent(),
        receipt.report(),
        receipt.class(),
        receipt.causal_evidence_digest(),
    )
    .expect("rebuild foreign vehicle receipt");
    refresh_baseline_authority(&mut changed_adapter);
    assert!(changed_adapter.validate().is_err());

    let mut changed_scenario = exact;
    let receipt = changed_scenario.promotion_baseline.terminal_receipts[0].clone();
    changed_scenario.promotion_baseline.terminal_receipts[0] =
        receipt_with_scenario_digest(&changed_scenario, &receipt, digest(95));
    refresh_baseline_authority(&mut changed_scenario);
    assert!(changed_scenario.validate().is_err());
}

fn promotion_snapshot(label: &str) -> JournalEvidenceSnapshot {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state,
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("open promotion campaign");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    tuner.run_promotion_once_blocking().expect("run promotion");
    tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("verify promotion snapshot")
}

fn sealed_snapshot(label: &str) -> JournalEvidenceSnapshot {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state,
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("open sealed campaign");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    tuner.run_promotion_once_blocking().expect("run promotion");
    tuner
        .run_final_qualification_once_blocking()
        .expect("run final qualification");
    tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("verify sealed snapshot")
}

fn refresh_head(snapshot: &mut JournalEvidenceSnapshot) {
    let bytes = serde_json::to_vec(&snapshot.head.entry).expect("encode changed journal head");
    snapshot.head.entry_digest = Digest::from_bytes(Sha256::digest(bytes).into());
}

fn receipt_with_scenario_digest(
    snapshot: &JournalEvidenceSnapshot,
    receipt: &RunTerminalReceipt,
    mission_content_digest: Digest,
) -> RunTerminalReceipt {
    let proof = &snapshot.promotion_baseline;
    let original = receipt.context();
    let scenario = MissionReference {
        revision_id: original.mission_revision_id().to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: mission_content_digest,
        max_samples: snapshot.stage.promotion_scenarios[0].max_samples,
        sample_timeout_ns: snapshot.stage.promotion_scenarios[0].sample_timeout_ns,
    };
    let context = RunExecutionContext::new(
        snapshot
            .head
            .entry
            .session
            .digest()
            .expect("digest session"),
        proof.trial_id,
        proof.role,
        proof.candidate_digest,
        None,
        ScenarioSet::Promotion,
        &scenario,
        original.repetition(),
        original.seed(),
        0,
    )
    .expect("create foreign scenario context");
    let RunTerminalSemanticOutcome::ScenarioComplete { run, .. } = receipt.intent().outcome()
    else {
        panic!("promotion receipt is not a completed scenario");
    };
    let outcome = RunTerminalSemanticOutcome::ScenarioComplete {
        candidate_digest: proof.candidate_digest,
        mission_content_digest,
        run: run.clone(),
    };
    rebuild_receipt(receipt, &context, outcome)
}

fn rebuild_receipt(
    original: &RunTerminalReceipt,
    context: &RunExecutionContext,
    outcome: RunTerminalSemanticOutcome,
) -> RunTerminalReceipt {
    let intent = RunTerminalIntent::new(
        context,
        context.digest().expect("digest foreign run context"),
        outcome,
    )
    .expect("create foreign terminal intent");
    let report = RunTerminalReport::new(
        original.report().plan(),
        &intent,
        original.report().recovery_state(),
        original.report().operations().to_vec(),
    )
    .expect("create foreign terminal report");
    let binding = RunBindingReceipt::new(
        context,
        original.report().plan(),
        original.binding().adapter().clone(),
    )
    .expect("create foreign run binding");
    let class = RunTerminalClass::classify(&intent, &report).expect("classify foreign receipt");
    RunTerminalReceipt::new(
        &binding,
        &intent,
        &report,
        class,
        original.causal_evidence_digest(),
    )
    .expect("create foreign terminal receipt")
}

fn refresh_baseline_authority(snapshot: &mut JournalEvidenceSnapshot) {
    refresh_proof(&mut snapshot.promotion_baseline);
    snapshot.promotion_closure.baseline_evaluation_digest =
        Some(snapshot.promotion_baseline.evaluation_digest);
    snapshot.promotion_closure.baseline_proof_digest =
        Some(snapshot.promotion_baseline.proof_digest);
    snapshot.promotion_closure.closure_digest = snapshot
        .promotion_closure
        .recompute_closure_digest()
        .expect("recompute promotion closure");
    let JournalEvent::PromotionClosed { closure } = &mut snapshot.head.entry.event else {
        panic!("promotion snapshot head");
    };
    *closure = snapshot.promotion_closure.clone();
    refresh_head(snapshot);
}

fn assert_proof_change_rejected(
    exact: &JournalEvidenceSnapshot,
    change: impl FnOnce(&mut flight_tune::AuthenticatedEvaluationProof),
) {
    let mut changed = exact.clone();
    change(&mut changed.promotion_baseline);
    refresh_proof(&mut changed.promotion_baseline);
    assert!(changed.validate().is_err());
}

fn assert_receipt_change_rejected(
    exact: &JournalEvidenceSnapshot,
    change: impl FnOnce(&mut flight_tune::AuthenticatedEvaluationProof),
) {
    let mut changed = exact.clone();
    change(&mut changed.promotion_baseline);
    refresh_proof(&mut changed.promotion_baseline);
    assert!(changed.promotion_baseline.validate().is_err());
    assert!(changed.validate().is_err());
}

fn refresh_proof(proof: &mut flight_tune::AuthenticatedEvaluationProof) {
    proof.evaluation_digest = proof
        .recompute_evaluation_digest()
        .expect("recompute changed evaluation");
    proof.proof_digest = proof
        .recompute_proof_digest()
        .expect("recompute changed proof");
}

fn assert_invalid_receipt_sequences(
    exact: &flight_tune::AuthenticatedEvaluationProof,
    foreign: &flight_tune::AuthenticatedEvaluationProof,
) {
    assert_receipt_proof_rejected(exact, |receipts| {
        receipts.pop();
    });
    assert_receipt_proof_rejected(exact, |receipts| {
        receipts.push(receipts[0].clone());
    });
    assert_receipt_proof_rejected(exact, |receipts| {
        receipts[1] = receipts[0].clone();
    });
    assert_receipt_proof_rejected(exact, |receipts| {
        receipts.swap(0, 1);
    });
    assert_receipt_proof_rejected(exact, |receipts| {
        receipts[0] = foreign.terminal_receipts[0].clone();
    });
}

fn assert_receipt_proof_rejected(
    exact: &flight_tune::AuthenticatedEvaluationProof,
    change: impl FnOnce(&mut Vec<flight_tune::RunTerminalReceipt>),
) {
    let mut changed = exact.clone();
    change(&mut changed.terminal_receipts);
    refresh_proof(&mut changed);
    assert!(changed.validate().is_err());
}

fn assert_closure_change_rejected(
    exact: &JournalEvidenceSnapshot,
    change: impl FnOnce(&mut flight_tune::PromotionClosure),
) {
    let mut changed = exact.clone();
    change(&mut changed.promotion_closure);
    assert!(changed.validate().is_err());
}

fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}

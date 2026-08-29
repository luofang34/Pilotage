use flight_tune::{AttemptRole, CandidateEvaluation, Digest, JournalEvent, PromotionDecision};

use super::{fixture, verify};

#[test]
fn unsupported_producer_schema_versions_fail_closed() {
    let mut snapshot = fixture::fixture();
    snapshot.journal.schema_version = snapshot.journal.schema_version.wrapping_add(1);
    assert!(verify(&snapshot).is_err());

    let mut policy = fixture::fixture();
    policy.journal.stage.promotion.schema_version = policy
        .journal
        .stage
        .promotion
        .schema_version
        .wrapping_add(1);
    assert!(verify(&policy).is_err());

    let mut authority = fixture::fixture();
    authority.journal.authority.schema_version =
        authority.journal.authority.schema_version.wrapping_add(1);
    assert!(verify(&authority).is_err());

    let mut proof = fixture::fixture();
    proof.journal.promotion_baseline.schema_version = proof
        .journal
        .promotion_baseline
        .schema_version
        .wrapping_add(1);
    fixture::refresh_proof(&mut proof.journal.promotion_baseline);
    fixture::refresh_promotion_authority(&mut proof);
    assert!(verify(&proof).is_err());

    let mut closure = fixture::fixture();
    closure.journal.promotion_closure.schema_version = closure
        .journal
        .promotion_closure
        .schema_version
        .wrapping_add(1);
    fixture::refresh_promotion_authority(&mut closure);
    assert!(verify(&closure).is_err());
}

#[test]
fn changed_policy_content_or_digest_fails() {
    let mut content = fixture::fixture();
    content.journal.stage.promotion.minimum_loss_improvement += 0.01;
    assert!(verify(&content).is_err());

    let mut identity = fixture::fixture();
    identity.journal.promotion_closure.policy_digest = Digest::from_bytes([99; 32]);
    fixture::refresh_promotion_authority(&mut identity);
    assert!(verify(&identity).is_err());
}

#[test]
fn changed_baseline_or_frozen_evaluation_fails_after_rechaining() {
    for frozen in [false, true] {
        let mut evidence = fixture::fixture();
        let proof = if frozen {
            evidence
                .journal
                .promotion_frozen
                .as_mut()
                .expect("frozen proof")
        } else {
            &mut evidence.journal.promotion_baseline
        };
        let CandidateEvaluation::Passed { aggregate, .. } = &mut proof.evaluation else {
            panic!("promotion proof must pass");
        };
        aggregate.mean_loss += 0.01;
        fixture::refresh_proof(proof);
        fixture::refresh_promotion_authority(&mut evidence);
        assert!(verify(&evidence).is_err(), "frozen={frozen}");
    }
}

#[test]
fn proof_trial_identity_cannot_define_the_expected_run_set() {
    let mut evidence = fixture::fixture();
    let saved = evidence
        .journal
        .promotion_frozen
        .as_ref()
        .expect("frozen proof");
    evidence.journal.promotion_frozen = Some(fixture::passing_proof(
        &evidence.journal.stage,
        &evidence.journal.head.entry.session,
        saved.trial_id.wrapping_add(10),
        AttemptRole::PromotionFrozen,
        saved.candidate_digest,
        0.80,
        0.35,
        0.21,
    ));
    fixture::rebuild_promotion_closure(&mut evidence);

    assert!(verify(&evidence).is_err());
}

#[test]
fn rechained_freeze_record_cannot_replace_candidate_authority() {
    let mut evidence = fixture::fixture();
    let replacement = Digest::from_bytes([202; 32]);
    evidence.journal.authority.frozen_candidate = replacement;
    let JournalEvent::Frozen { candidate, .. } = &mut evidence.journal.authority.frozen.entry.event
    else {
        panic!("freeze authority event");
    };
    *candidate = replacement;
    evidence.journal.authority.frozen.entry_digest =
        crate::digest::document("journal entry", &evidence.journal.authority.frozen.entry)
            .expect("rechain freeze authority");

    assert!(verify(&evidence).is_err());
}

#[test]
fn removing_all_promotion_receipts_fails_after_aggregate_recompute() {
    let mut evidence = fixture::fixture();
    for proof in [
        &mut evidence.journal.promotion_baseline,
        evidence
            .journal
            .promotion_frozen
            .as_mut()
            .expect("frozen proof"),
    ] {
        let CandidateEvaluation::Passed { aggregate, runs } = &mut proof.evaluation else {
            panic!("promotion proof must pass");
        };
        *aggregate = super::super::statistics::aggregate(runs).expect("recompute aggregate");
        proof.terminal_receipts.clear();
        fixture::refresh_proof(proof);
    }
    fixture::refresh_promotion_authority(&mut evidence);
    assert!(verify(&evidence).is_err());
}

#[test]
fn a_foreign_promotion_receipt_fails_after_rechaining() {
    let mut evidence = fixture::fixture();
    let foreign = evidence.journal.promotion_baseline.terminal_receipts[0].clone();
    let frozen = evidence
        .journal
        .promotion_frozen
        .as_mut()
        .expect("frozen proof");
    frozen.terminal_receipts[0] = foreign;
    fixture::refresh_proof(frozen);
    fixture::refresh_promotion_authority(&mut evidence);
    assert!(verify(&evidence).is_err());
}

#[test]
fn stored_promotion_and_selection_cannot_override_recomputation() {
    let mut promoted = fixture::fixture();
    install_worse_frozen(&mut promoted);
    fixture::rebuild_promotion_closure(&mut promoted);
    assert!(matches!(
        promoted.journal.promotion_closure.decision,
        PromotionDecision::RejectedNoImprovement { .. }
    ));
    let frozen_candidate = promoted
        .journal
        .promotion_frozen
        .as_ref()
        .expect("frozen proof")
        .candidate_digest;
    promoted.journal.promotion_closure.decision = PromotionDecision::Promoted {};
    promoted.journal.promotion_closure.selected_candidate = Some(frozen_candidate);
    fixture::refresh_promotion_authority(&mut promoted);
    assert_promotion_recompute_error(&promoted);

    let mut selection = fixture::fixture();
    install_worse_frozen(&mut selection);
    fixture::rebuild_promotion_closure(&mut selection);
    selection.journal.promotion_closure.selected_candidate = Some(frozen_candidate);
    fixture::refresh_promotion_authority(&mut selection);
    assert_promotion_recompute_error(&selection);
}

fn install_worse_frozen(evidence: &mut crate::CampaignEvidence) {
    let current = evidence
        .journal
        .promotion_frozen
        .as_ref()
        .expect("frozen proof");
    evidence.journal.promotion_frozen = Some(fixture::passing_proof(
        &evidence.journal.stage,
        &evidence.journal.head.entry.session,
        current.trial_id,
        AttemptRole::PromotionFrozen,
        current.candidate_digest,
        1.20,
        0.35,
        0.21,
    ));
}

fn assert_promotion_recompute_error(evidence: &crate::CampaignEvidence) {
    assert!(verify(evidence).is_err());
}

#[test]
fn a_qualification_policy_that_names_no_objective_limit_is_refused() {
    // Checked against the stage verifier directly rather than through a
    // published document: clearing the map in sealed evidence breaks a digest,
    // so the whole document is refused for a reason that has nothing to do
    // with the policy, and the case would pass whether or not the policy is
    // checked at all.
    //
    // Without this check a campaign published under an empty final
    // qualification bar verifies: every digest reconciles, nothing is found
    // over limit, and the evidence reads as qualified. That is a valid chain
    // attesting a bar nobody set.
    let sealed = fixture::fixture();
    let mut stage = sealed.journal.stage.clone();
    assert!(
        super::super::stage::verify(&stage).is_ok(),
        "the fixture's own stage is valid to begin with"
    );

    stage.qualification.objectives.clear();
    assert!(
        super::super::stage::verify(&stage).is_err(),
        "an empty qualification bar is not a bar"
    );

    // The promotion half already refused this; the two are now symmetric.
    let mut promotion = sealed.journal.stage.clone();
    promotion.promotion.objectives.clear();
    assert!(super::super::stage::verify(&promotion).is_err());
}

/// The independent verifier restates the crash-gate floor.
///
/// A campaign published with the crash gate dropped or moved would reconcile
/// perfectly against its own chain: every digest matches and nothing is found
/// over limit. What it would not have is the gate that makes a measurement
/// mean anything.
#[test]
fn a_stage_that_moves_or_drops_the_crash_gate_is_refused() {
    let sealed = fixture::fixture();
    let stage = sealed.journal.stage.clone();
    assert!(
        super::super::stage::verify(&stage).is_ok(),
        "the fixture's own stage is valid to begin with"
    );

    let mut omitted = stage.clone();
    omitted.required_hard_gates = vec!["finite".to_owned()];
    assert!(super::super::stage::verify(&omitted).is_err());

    let mut renamed = stage.clone();
    renamed.required_hard_gates = vec!["crash".to_owned(), "finite".to_owned()];
    assert!(super::super::stage::verify(&renamed).is_err());

    let mut reordered = stage;
    reordered.required_hard_gates = vec![
        "finite".to_owned(),
        flight_tune::MANDATORY_CRASH_GATE_ID.to_owned(),
    ];
    assert!(super::super::stage::verify(&reordered).is_err());
}

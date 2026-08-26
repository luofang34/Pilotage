#![allow(clippy::expect_used, clippy::panic)]

mod attacks;
mod fixture;
#[path = "../../../../tools/flight-tune/tests/tuner/test_rig.rs"]
#[allow(dead_code)]
mod producer_rig;

use flight_tune::{CandidateEvaluation, FinalQualificationOutcome};

use crate::{CampaignEvidence, VerifiedCampaignEvidence, digest};

use fixture::{fixture, refresh_head, refresh_proof};
use producer_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    TestDirectory, candidate, stage,
};

/// The bar a campaign's own stage states. The cases below are about the
/// chain rather than the bar, so they pin the bar the producer used; the
/// cases about the bar itself pin a different one on purpose.
fn stated_policy(evidence: &CampaignEvidence) -> crate::RequiredPolicy {
    crate::RequiredPolicy::new(
        &evidence.journal.stage.promotion,
        &evidence.journal.stage.qualification,
    )
    .expect("bind the stated policy")
}

fn verify(evidence: &CampaignEvidence) -> Result<VerifiedCampaignEvidence, crate::FeedbackError> {
    let bytes = digest::encode("campaign evidence", evidence)?;
    VerifiedCampaignEvidence::from_bytes(&bytes, digest::hash(&bytes))
}

#[test]
fn the_sealed_journal_chain_is_dense_and_linked() {
    // The tamper cases mutate this chain and require the verifier to refuse
    // it. A fixture whose chain was already broken would make every one of
    // them pass without the defence they name ever running, so the
    // unmutated chain is asserted here first: dense from zero, each entry
    // hash-linked to the one before, every entry digest recomputing, and the
    // head the last record.
    let evidence = fixture();
    fixture::assert_journal_chain_linked(&evidence);
}

#[test]
fn sealed_golden_evidence_qualifies() {
    let evidence = fixture();
    let verified = verify(&evidence).expect("verify golden evidence");
    let selected = verified.selected_candidate().expect("selected candidate");
    assert_eq!(
        verified.outcome(),
        Some(&FinalQualificationOutcome::Qualified)
    );
    assert_eq!(
        verified
            .clone()
            .verify_qualified(&stated_policy(&evidence))
            .expect("verify qualified evidence")
            .selected_candidate(),
        selected
    );
}

#[test]
fn journal_producer_snapshot_qualifies_independently() {
    let directory = TestDirectory::new("feedback-producer");
    let state = FakeHandle::new();
    let mut tuner = flight_tune::Tuner::open_or_resume(
        directory.path(),
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::new(2.0),
        QuadraticMetric::new(state),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("open producer tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run producer training");
    tuner.freeze_candidate().expect("freeze producer candidate");
    tuner
        .run_promotion_once_blocking()
        .expect("run producer promotion");
    assert_eq!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("run producer final qualification"),
        FinalQualificationOutcome::Qualified
    );
    let snapshot = tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("read producer evidence snapshot");
    let evidence = CampaignEvidence::new(snapshot).expect("verify producer snapshot");
    let required = stated_policy(&evidence);
    assert!(
        verify(&evidence)
            .and_then(|verified| verified.verify_qualified(&required))
            .is_ok()
    );
}

#[test]
fn canonical_source_digest_is_fixed() {
    let evidence = fixture();
    let bytes = digest::encode("campaign evidence", &evidence).expect("encode evidence");
    let verified = VerifiedCampaignEvidence::from_bytes(&bytes, digest::hash(&bytes))
        .expect("verify evidence");
    assert_eq!(
        verified.source_digest().to_string(),
        "fd8f60cca54d07428cf39b5b7f4784beaec6d2016c429314f2c28e2480f8f303"
    );
}

#[test]
fn noncanonical_json_is_rejected() {
    let evidence = fixture();
    let mut bytes = digest::encode("campaign evidence", &evidence).expect("encode evidence");
    bytes.push(b'\n');
    let expected = digest::hash(&bytes[..bytes.len().saturating_sub(1)]);
    assert!(VerifiedCampaignEvidence::from_bytes(&bytes, expected).is_err());
}

#[test]
fn missing_extra_repeated_and_swapped_pairs_are_rejected() {
    for mutation in 0..4 {
        let mut evidence = fixture();
        let proof = evidence
            .journal
            .promotion_frozen
            .as_mut()
            .expect("frozen proof");
        let CandidateEvaluation::Passed { aggregate, runs } = &mut proof.evaluation else {
            panic!("frozen proof must pass");
        };
        match mutation {
            0 => {
                proof.terminal_receipts.pop();
                runs.pop();
            }
            1 => {
                proof
                    .terminal_receipts
                    .push(proof.terminal_receipts[0].clone());
                runs.push(runs[0].clone());
            }
            2 => {
                proof.terminal_receipts[1] = proof.terminal_receipts[0].clone();
                runs[1] = runs[0].clone();
            }
            3 => {
                proof.terminal_receipts.swap(0, 1);
                runs.swap(0, 1);
            }
            _ => panic!("mutation index is bounded"),
        };
        *aggregate = super::statistics::aggregate(runs.as_slice()).expect("aggregate changed runs");
        refresh_proof(proof);
        fixture::refresh_promotion_authority(&mut evidence);
        assert!(verify(&evidence).is_err(), "mutation {mutation} passed");
    }
}

#[test]
fn one_pair_cannot_omit_an_objective() {
    let mut evidence = fixture();
    let proof = evidence.journal.promotion_baseline.clone();
    evidence.journal.promotion_baseline = fixture::proof_with_missing_objective(
        &evidence.journal.stage,
        &evidence.journal.head.entry.session,
        proof.trial_id,
        proof.role,
        proof.candidate_digest,
    );
    fixture::refresh_promotion_authority(&mut evidence);
    assert!(verify(&evidence).is_err());
}

#[test]
fn every_run_needs_the_exact_ordered_hard_gates() {
    for hard_gates in [
        &["crash"][..],
        &["crash", "finite", "bounded"][..],
        &["finite", "crash"][..],
    ] {
        let mut evidence = fixture();
        let proof = evidence.journal.promotion_baseline.clone();
        evidence.journal.promotion_baseline = fixture::proof_with_hard_gates(
            &evidence.journal.stage,
            &evidence.journal.head.entry.session,
            proof.trial_id,
            proof.role,
            proof.candidate_digest,
            hard_gates,
        );
        fixture::refresh_promotion_authority(&mut evidence);
        assert!(
            verify(&evidence).is_err(),
            "hard-gate mutation {hard_gates:?} passed"
        );
    }
}

#[test]
fn a_quarantine_cannot_keep_a_paired_comparison() {
    let mut evidence = fixture();
    let frozen = evidence
        .journal
        .promotion_frozen
        .as_ref()
        .expect("frozen proof");
    evidence.journal.promotion_frozen = Some(fixture::quarantined_proof(
        &evidence.journal.stage,
        &evidence.journal.head.entry.session,
        frozen.trial_id,
        frozen.role,
        frozen.candidate_digest,
    ));
    fixture::refresh_promotion_authority(&mut evidence);
    assert!(verify(&evidence).is_err());
}

#[test]
fn every_final_scenario_requires_every_repetition() {
    let mut evidence = fixture();
    let repetitions = evidence.journal.stage.repetitions as usize;
    let proof = evidence.journal.final_proof.as_mut().expect("final proof");
    let CandidateEvaluation::Passed { aggregate, runs } = &mut proof.evaluation else {
        panic!("final proof must pass");
    };
    proof.terminal_receipts = proof
        .terminal_receipts
        .iter()
        .step_by(repetitions)
        .cloned()
        .collect();
    *runs = runs.iter().step_by(repetitions).cloned().collect();
    assert_eq!(runs.len(), 2);
    *aggregate = super::statistics::aggregate(runs.as_slice()).expect("aggregate reduced final");
    refresh_proof(proof);
    refresh_head(&mut evidence);
    assert!(verify(&evidence).is_err());
}

#[test]
fn content_addressed_store_has_exact_readback() {
    let temporary = tempfile::tempdir().expect("create temporary directory");
    let root = temporary
        .path()
        .canonicalize()
        .expect("canonicalize temporary directory")
        .join("evidence");
    let evidence = fixture();
    let receipt = evidence
        .store_content_addressed_blocking(root)
        .expect("store evidence");
    let loaded = VerifiedCampaignEvidence::load_content_addressed_blocking(&receipt.object_path)
        .expect("load evidence");
    assert_eq!(loaded.source_digest(), receipt.digest);
}

#[test]
fn a_campaign_run_against_another_bar_does_not_qualify() {
    // The evidence states the policy its own operator chose, so a verifier
    // that reads the bar out of the document it is checking can only attest
    // self-consistency. A campaign run against limits nobody set reconciles
    // exactly as well as one run against the real limits.
    let evidence = fixture();
    let verified = verify(&evidence).expect("verify evidence");

    // The bar the producer actually used qualifies it.
    assert!(
        verified
            .clone()
            .verify_qualified(&stated_policy(&evidence))
            .is_ok()
    );

    // A stricter final qualification bar is a different bar, and this
    // campaign was not run against it.
    let mut stricter = evidence.journal.stage.qualification.clone();
    for maximum in stricter.objective_maxima.values_mut() {
        *maximum /= 2.0;
    }
    let required = crate::RequiredPolicy::new(&evidence.journal.stage.promotion, &stricter)
        .expect("bind a stricter policy");
    assert!(
        verified.clone().verify_qualified(&required).is_err(),
        "a campaign must not qualify against a bar it never ran against"
    );

    // So is a different promotion bar.
    let mut looser = evidence.journal.stage.promotion.clone();
    looser.minimum_relative_loss_improvement = 0.0;
    let required = crate::RequiredPolicy::new(&looser, &evidence.journal.stage.qualification)
        .expect("bind a different promotion policy");
    assert!(verified.verify_qualified(&required).is_err());
}

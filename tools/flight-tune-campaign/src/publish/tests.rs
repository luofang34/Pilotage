#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../flight-tune/tests/tuner/test_rig.rs"]
#[allow(dead_code)]
mod producer_rig;

use flight_tune::{ArtifactIdentity, Digest, FinalQualificationOutcome, Tuner};
use pilotage_tuning_feedback::{CampaignEvidence, RequiredPolicy, VerifiedCampaignEvidence};

use super::publish_journal_evidence_blocking;
use crate::CampaignError;
use producer_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    TestDirectory, candidate, stage,
};

#[test]
fn an_unstable_head_is_rejected_before_storage() {
    let directory = TestDirectory::new("campaign-unstable-publish");
    let state = FakeHandle::new();
    let tuner = Tuner::open_or_resume(
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
    let evidence_root = directory.path().join("published");

    let error = publish_journal_evidence_blocking(tuner.journal(), &evidence_root)
        .expect_err("reject unstable journal head");

    assert!(matches!(error, CampaignError::Snapshot { .. }));
    assert!(!evidence_root.exists());
}

#[test]
fn a_qualified_journal_publishes_verified_readback() {
    let directory = TestDirectory::new("campaign-qualified-publish");
    let state = FakeHandle::new();
    let mut tuner = Tuner::open_or_resume(
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
    let evidence_root = directory.path().join("published");

    let receipt = publish_journal_evidence_blocking(tuner.journal(), evidence_root)
        .expect("publish qualified evidence");
    // The bar is named by the consumer, not read out of the document being
    // checked. This campaign runs the rig's own stage, so the rig's stage is
    // the bar it must be held to.
    let rig_stage = stage();
    let required = RequiredPolicy::new(&rig_stage.promotion, &rig_stage.qualification)
        .expect("bind the rig's policy");
    let qualified = VerifiedCampaignEvidence::load_content_addressed_blocking(&receipt.object_path)
        .and_then(|verified| verified.verify_qualified(&required))
        .expect("verify published readback");

    assert_eq!(qualified.campaign().source_digest(), receipt.digest);
    assert!(!qualified.selected_candidate().is_zero());
}

#[test]
fn a_changed_scenario_runtime_identity_fails_independent_verification() {
    let directory = TestDirectory::new("campaign-runtime-identity-tamper");
    let state = FakeHandle::new();
    let mut tuner = Tuner::open_or_resume(
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

    let exact = tuner
        .journal()
        .verified_evidence_snapshot()
        .expect("a stable journal head");
    CampaignEvidence::new(exact.clone()).expect("the exact evidence verifies");

    // The scenario runtime identity is inside the entry the journal head
    // digest covers, so a reader that recalculates the head finds it
    // whatever the rest of the campaign says.
    let bound = exact
        .head
        .entry
        .session
        .runtimes
        .scenario_runtime
        .clone()
        .expect("the bound scenario runtime identity");
    let mut changed = exact.clone();
    changed.head.entry.session.runtimes.scenario_runtime = Some(
        ArtifactIdentity::new(
            "pilotage-scenario-runtime-v2",
            Digest::from_bytes([0x5a; 32]),
        )
        .expect("another runtime identity"),
    );
    assert_ne!(
        changed
            .head
            .entry
            .session
            .runtimes
            .scenario_runtime
            .as_ref(),
        Some(&bound)
    );
    CampaignEvidence::new(changed)
        .expect_err("a changed runtime identity must fail independent verification");

    // Removing the identity is refused too: evidence that names no runtime
    // is evidence nobody can reproduce.
    let mut absent = exact;
    absent.head.entry.session.runtimes.scenario_runtime = None;
    CampaignEvidence::new(absent)
        .expect_err("evidence with no runtime identity must fail independent verification");
}

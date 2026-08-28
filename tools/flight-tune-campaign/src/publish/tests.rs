#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../flight-tune/tests/tuner/test_rig.rs"]
#[allow(dead_code)]
mod producer_rig;

use flight_tune::{FinalQualificationOutcome, Tuner};
use pilotage_tuning_feedback::{RequiredPolicy, VerifiedCampaignEvidence};

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

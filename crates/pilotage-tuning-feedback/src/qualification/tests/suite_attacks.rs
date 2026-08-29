//! Attacks on the suite the independent verifier derives for itself.
//!
//! A campaign states which search group a challenger changed. That statement
//! is not evidence. The group follows from the parameters that differ between
//! the incumbent and the challenger, so each case below states a different
//! group and requires the verifier to derive the real one and refuse.

use flight_tune::{FinalQualificationOutcome, JournalEvent};

use crate::CampaignEvidence;

use super::producer_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, ParameterSequenceStrategy,
    QuadraticMetric, TestDirectory, two_group_candidate, two_group_stage,
};
use super::{stated_policy, verify};

/// A sealed campaign that ran one challenger in each of two search groups.
fn sealed_campaign(name: &str) -> CampaignEvidence {
    let directory = TestDirectory::new(name);
    let state = FakeHandle::new();
    let mut tuner = flight_tune::Tuner::open_or_resume(
        directory.path(),
        two_group_stage(),
        91,
        two_group_candidate(0.0, 0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::new(2.0),
        QuadraticMetric::new(state),
        ParameterSequenceStrategy::new(vec![vec![("gain", 0.5)], vec![("trim", 0.5)]]),
    )
    .expect("open a two group producer tuner");
    tuner
        .run_training_attempts_blocking(2)
        .expect("run one challenger in each group");
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
    CampaignEvidence::new(snapshot).expect("verify the producer snapshot")
}

/// Applies one change to the first recorded transition, then relinks the chain
/// so the refusal is the derived suite and not the link check.
fn change_first_transition(
    evidence: &mut CampaignEvidence,
    change: impl Fn(&mut flight_tune::SearchGroupBinding),
) {
    let mut applied = false;
    for record in &mut evidence.journal.authority.journal_chain {
        if let JournalEvent::CandidateTransitionAuthorized { group, .. } = &mut record.entry.event
            && !applied
        {
            change(group);
            applied = true;
        }
    }
    assert!(applied, "the producer recorded a transition");
    super::retry_attacks::rechain(evidence);
}

fn assert_refused(evidence: &CampaignEvidence, case: &str) {
    let error = verify(evidence)
        .err()
        .unwrap_or_else(|| panic!("the verifier accepted a changed {case}"));
    assert!(
        !format!("{error}").contains("chain changed"),
        "the {case} case was refused before the suite was derived: {error}"
    );
}

#[test]
fn an_untouched_two_group_campaign_qualifies() {
    let evidence = sealed_campaign("suite-attack-baseline");
    let required = stated_policy(&evidence);

    verify(&evidence)
        .and_then(|verified| verified.verify_qualified(&required))
        .expect("an untouched two group campaign qualifies");
}

#[test]
fn a_substituted_training_suite_is_refused() {
    let mut evidence = sealed_campaign("suite-attack-substitution");
    let other = evidence.journal.stage.training_suites[1].clone();
    change_first_transition(&mut evidence, |group| {
        group.suite_id = other.id.clone();
        group.suite_index = 1;
        group.suite_digest = other.digest().expect("the other suite digest");
    });

    assert_refused(&evidence, "training suite");
}

#[test]
fn a_substituted_search_group_is_refused() {
    let mut evidence = sealed_campaign("suite-attack-group");
    change_first_transition(&mut evidence, |group| {
        group.group_id = "trim-group".to_owned();
    });

    assert_refused(&evidence, "search group");
}

#[test]
fn a_changed_suite_digest_is_refused() {
    let mut evidence = sealed_campaign("suite-attack-digest");
    change_first_transition(&mut evidence, |group| {
        group.suite_digest = flight_tune::Digest::from_bytes([9; 32]);
    });

    assert_refused(&evidence, "suite digest");
}

#[test]
fn a_changed_candidate_parameter_is_refused() {
    let mut evidence = sealed_campaign("suite-attack-candidate");
    // A candidate the chain never named, so its digest cannot be the one
    // the transition recorded.
    let changed = two_group_candidate(0.9, 0.9);
    let last = evidence
        .journal
        .authority
        .candidates
        .len()
        .saturating_sub(1);
    evidence.journal.authority.candidates[last] = changed;

    assert_refused(&evidence, "candidate parameter");
}

#[test]
fn an_incomplete_candidate_list_is_refused() {
    let mut evidence = sealed_campaign("suite-attack-candidate-count");
    evidence.journal.authority.candidates.pop();

    assert_refused(&evidence, "candidate list");
}

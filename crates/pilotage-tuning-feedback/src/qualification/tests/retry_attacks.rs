//! Attacks on the execution retry and quarantine relation.
//!
//! Each case starts from a sealed campaign that a real runner produced with
//! one real quarantine and one real replacement in it, changes exactly one
//! thing, and requires the independent verifier to refuse the result.

use flight_tune::{
    AttemptProjectionOutcome, AttemptRetryOutcome, Digest, FinalQualificationOutcome, JournalEvent,
};

use crate::{CampaignEvidence, VerifiedCampaignEvidence, digest};

use super::producer_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    TestDirectory, candidate, stage_with_execution_retry_limit,
};
use super::{stated_policy, verify};

/// A sealed campaign whose training baseline was quarantined and replaced.
///
/// The quarantine comes out of a failing simulator start rather than out of a
/// hand-written event, so every identity below is one the producer actually
/// bound.
fn replaced_campaign(name: &str) -> CampaignEvidence {
    let directory = TestDirectory::new(name);
    let state = FakeHandle::new();
    state.0.borrow_mut().fail_starts_through = 1;
    let mut tuner = flight_tune::Tuner::open_or_resume(
        directory.path(),
        stage_with_execution_retry_limit(1),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::new(2.0),
        QuadraticMetric::new(state),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("open a retrying producer tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run producer training through one replacement");
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

/// Requires the evidence to survive its own bar unchanged.
fn assert_qualifies(evidence: &CampaignEvidence) {
    let required = stated_policy(evidence);
    verify(evidence)
        .and_then(|verified| verified.verify_qualified(&required))
        .expect("an untouched replaced campaign qualifies");
}

/// Requires the evidence to be refused, whichever door it is offered at.
fn assert_refused(evidence: &CampaignEvidence) {
    let error = verify(evidence).err().expect("a refusal");
    // A refusal that only reports a broken chain link proves the link check,
    // not the relation each case is about.
    assert!(
        !format!("{error}").contains("chain changed"),
        "the case was refused before the retry relation was read: {error}"
    );
}

/// Relinks a chain a case has changed, so its refusal cannot be the link check.
///
/// Sequence numbers, previous digests, the head, and the named records are all
/// made consistent again. The stored projection is left exactly as the
/// producer wrote it unless the case changed it on purpose.
pub(super) fn rechain(evidence: &mut CampaignEvidence) {
    let mut previous = None;
    for (index, record) in evidence
        .journal
        .authority
        .journal_chain
        .iter_mut()
        .enumerate()
    {
        record.entry.sequence = u64::try_from(index).expect("journal sequence");
        record.entry.previous = previous;
        record.entry_digest =
            digest::document("journal entry", &record.entry).expect("journal entry digest");
        previous = Some(record.entry_digest);
    }
    let head = evidence
        .journal
        .authority
        .journal_chain
        .last()
        .expect("journal head")
        .clone();
    evidence.journal.head = flight_tune::AuthenticatedJournalHead {
        entry: head.entry,
        entry_digest: head.entry_digest,
    };
    sync_named_records(evidence);
}

/// Rebuilds the stored projection from a chain a case has changed.
///
/// Without this the projection equality check answers every chain case, and
/// the rules about what a retry relation may say would never be reached.
fn resync_projection(evidence: &mut CampaignEvidence) {
    let limit = evidence.journal.stage.execution_retry.execution_retry_limit;
    if let Ok(projection) = flight_tune::AttemptProjection::from_journal_chain(
        &evidence.journal.authority.journal_chain,
        limit,
    ) {
        evidence.journal.authority.attempts = projection;
    }
}

fn sync_named_records(evidence: &mut CampaignEvidence) {
    let chain = evidence.journal.authority.journal_chain.clone();
    let find = |role: flight_tune::AttemptRole| {
        chain
            .iter()
            .filter(|record| {
                matches!(
                    &record.entry.event,
                    JournalEvent::AttemptPrepared { role: saved, .. } if *saved == role
                )
            })
            .next_back()
            .cloned()
            .expect("a named attempt record")
    };
    evidence.journal.authority.frozen = chain
        .iter()
        .find(|record| matches!(record.entry.event, JournalEvent::Frozen { .. }))
        .cloned()
        .expect("the freeze record");
    evidence.journal.authority.promotion_baseline =
        find(flight_tune::AttemptRole::PromotionBaseline);
    evidence.journal.authority.promotion_frozen =
        Some(find(flight_tune::AttemptRole::PromotionFrozen));
    evidence.journal.authority.final_qualification =
        Some(find(flight_tune::AttemptRole::FinalQualification));
}

/// The index of the one quarantined record in the stored projection.
fn quarantined_index(evidence: &CampaignEvidence) -> usize {
    evidence
        .journal
        .authority
        .attempts
        .attempts
        .iter()
        .position(|record| matches!(record.outcome, AttemptProjectionOutcome::Quarantined { .. }))
        .expect("one quarantined record")
}

#[test]
fn a_real_replacement_campaign_seals_and_qualifies() {
    let evidence = replaced_campaign("retry-attack-baseline");

    // The relation the producer built is the one the cases below attack, so
    // a case that refuses it is refusing the change and not the fixture.
    let attempts = &evidence.journal.authority.attempts;
    assert_eq!(attempts.execution_retry_limit, 1);
    let quarantined = &attempts.attempts[quarantined_index(&evidence)];
    assert_eq!(quarantined.retry_index, 0);
    let AttemptProjectionOutcome::Quarantined { retry, .. } = quarantined.outcome else {
        panic!("the producer quarantined one attempt");
    };
    let AttemptRetryOutcome::Authorized {
        replacement_trial_id,
        replacement_retry_index,
    } = retry
    else {
        panic!("the declared limit authorized one replacement");
    };
    assert_eq!(replacement_retry_index, 1);
    assert!(
        attempts
            .attempts
            .iter()
            .any(|record| record.trial_id == replacement_trial_id && record.retry_index == 1)
    );

    assert_qualifies(&evidence);
}

#[test]
fn a_replacement_that_changes_the_seed_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-seed");
    let mut changed = false;
    for record in &mut evidence.journal.authority.journal_chain {
        if let JournalEvent::RunPrepared { context, .. } = &mut record.entry.event
            && context.retry_index() == 1
        {
            // The seed is the experimental condition. A replacement that
            // draws a different one is a different experiment.
            *context = rebuilt_with_seed(context, context.seed().wrapping_add(1));
            changed = true;
        }
    }
    assert!(changed, "the replacement prepared at least one run");
    rechain(&mut evidence);
    assert_refused(&evidence);
}

#[test]
fn a_replacement_that_changes_its_scenario_or_repetition_is_refused() {
    for repetition in [7_u32, 9] {
        let mut evidence = replaced_campaign(&format!("retry-attack-repetition-{repetition}"));
        for record in &mut evidence.journal.authority.journal_chain {
            if let JournalEvent::RunPrepared { context, .. } = &mut record.entry.event
                && context.retry_index() == 1
            {
                *context = rebuilt_with_repetition(context, repetition);
            }
        }
        rechain(&mut evidence);
        assert_refused(&evidence);
    }
}

#[test]
fn a_projection_with_an_orphan_quarantine_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-orphan");
    let index = quarantined_index(&evidence);
    let orphan = evidence.journal.authority.attempts.attempts[index].clone();
    // A quarantine the chain never carried cannot enter through the summary
    // that is supposed to describe the chain.
    evidence.journal.authority.attempts.attempts.push(orphan);
    assert_refused(&evidence);
}

#[test]
fn a_projection_that_omits_a_journal_quarantine_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-omitted");
    let index = quarantined_index(&evidence);
    evidence.journal.authority.attempts.attempts.remove(index);
    assert_refused(&evidence);
}

#[test]
fn a_projection_that_claims_early_exhaustion_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-early-exhaustion");
    let index = quarantined_index(&evidence);
    // The source sits at retry index zero under a limit of one, so the
    // declared limit still owes it a replacement.
    if let AttemptProjectionOutcome::Quarantined { retry, .. } =
        &mut evidence.journal.authority.attempts.attempts[index].outcome
    {
        *retry = AttemptRetryOutcome::Exhausted { retry_index: 0 };
    }
    assert_refused(&evidence);
}

#[test]
fn a_projection_that_invents_a_quarantine_reason_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-reason");
    let index = quarantined_index(&evidence);
    if let AttemptProjectionOutcome::Quarantined { reason_digest, .. } =
        &mut evidence.journal.authority.attempts.attempts[index].outcome
    {
        *reason_digest = Digest::from_bytes([77; 32]);
    }
    assert_refused(&evidence);
}

#[test]
fn two_replacements_cannot_answer_one_source() {
    let mut evidence = replaced_campaign("retry-attack-two-replacements");
    let index = quarantined_index(&evidence);
    let mut duplicate = evidence.journal.authority.attempts.attempts[index].clone();
    duplicate.trial_id = duplicate.trial_id.wrapping_add(64);
    evidence.journal.authority.attempts.attempts.push(duplicate);
    assert_refused(&evidence);
}

#[test]
fn a_missing_replacement_receipt_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-missing-receipt");
    let mut removed = false;
    evidence.journal.authority.journal_chain.retain(|record| {
        let drop = matches!(
            &record.entry.event,
            JournalEvent::RunCommitted { receipt, .. }
                if receipt.context().retry_index() == 1
        ) && !removed;
        if drop {
            removed = true;
        }
        !drop
    });
    assert!(removed, "the replacement committed at least one receipt");
    rechain(&mut evidence);
    assert_refused(&evidence);
}

#[test]
fn a_retry_index_above_the_declared_limit_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-over-limit");
    let index = quarantined_index(&evidence);
    if let AttemptProjectionOutcome::Quarantined { retry, .. } =
        &mut evidence.journal.authority.attempts.attempts[index].outcome
    {
        *retry = AttemptRetryOutcome::Authorized {
            replacement_trial_id: 64,
            replacement_retry_index: 9,
        };
    }
    assert_refused(&evidence);
}

#[test]
fn a_chain_retry_index_that_is_not_derived_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-chain-index");
    for record in &mut evidence.journal.authority.journal_chain {
        if let JournalEvent::RetryAuthorized { retry_index, .. } = &mut record.entry.event {
            // The replacement's place in the chain is one past its source's.
            // Nothing the authorization says can move it.
            *retry_index = 5;
        }
    }
    rechain(&mut evidence);
    resync_projection(&mut evidence);
    assert_refused(&evidence);
}

#[test]
fn a_chain_retry_that_names_another_replacement_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-chain-replacement");
    for record in &mut evidence.journal.authority.journal_chain {
        if let JournalEvent::RetryAuthorized {
            replacement_trial_id,
            ..
        } = &mut record.entry.event
        {
            *replacement_trial_id = replacement_trial_id.wrapping_add(9);
        }
    }
    rechain(&mut evidence);
    resync_projection(&mut evidence);
    assert_refused(&evidence);
}

#[test]
fn a_chain_retry_that_invents_a_reason_is_refused() {
    let mut evidence = replaced_campaign("retry-attack-chain-reason");
    for record in &mut evidence.journal.authority.journal_chain {
        if let JournalEvent::RetryAuthorized {
            quarantine_reason_digest,
            ..
        } = &mut record.entry.event
        {
            *quarantine_reason_digest = Digest::from_bytes([53; 32]);
        }
    }
    rechain(&mut evidence);
    resync_projection(&mut evidence);
    assert_refused(&evidence);
}

#[test]
fn a_campaign_that_replaced_an_execution_fails_a_no_retry_bar() {
    let evidence = replaced_campaign("retry-attack-bar");
    let bytes = digest::encode("campaign evidence", &evidence).expect("encode evidence");
    let verified = VerifiedCampaignEvidence::from_bytes(&bytes, digest::hash(&bytes))
        .expect("verify replaced evidence");
    let strict = crate::RequiredPolicy::new(
        &evidence.journal.stage.promotion,
        &evidence.journal.stage.qualification,
        &flight_tune::ExecutionRetryPolicy::none(),
    )
    .expect("bind a no-retry bar");

    assert!(
        verified.verify_qualified(&strict).is_err(),
        "a campaign that discarded an execution must not clear a bar that forbids one"
    );
}

/// Rebuilds one run identity with a different seed and nothing else changed.
fn rebuilt_with_seed(
    context: &flight_tune::RunExecutionContext,
    seed: u64,
) -> flight_tune::RunExecutionContext {
    rebuild(context, context.repetition(), seed)
}

/// Rebuilds one run identity with a different repetition and nothing else.
fn rebuilt_with_repetition(
    context: &flight_tune::RunExecutionContext,
    repetition: u32,
) -> flight_tune::RunExecutionContext {
    rebuild(context, repetition, context.seed())
}

fn rebuild(
    context: &flight_tune::RunExecutionContext,
    repetition: u32,
    seed: u64,
) -> flight_tune::RunExecutionContext {
    let mission = flight_tune::MissionReference {
        revision_id: context.mission_revision_id().to_owned(),
        schema_version: flight_tune::MISSION_SCHEMA_VERSION,
        content_digest: context.mission_content_digest(),
        max_samples: super::producer_rig::FAKE_MAX_SAMPLES,
        sample_timeout_ns: 20_000_000,
    };
    flight_tune::RunExecutionContext::new(
        context.tuning_session_digest(),
        context.trial_id(),
        context.role(),
        context.candidate_digest(),
        context.transition_authorization(),
        context.scenario_set(),
        &mission,
        repetition,
        seed,
        context.retry_index(),
    )
    .expect("rebuild one changed run identity")
}

//! Execution retry and quarantine, proved end to end against a live campaign.
//!
//! Every case here drives a real quarantine out of the runner rather than
//! writing one into the journal, so a case that claims a replacement ran is
//! reading the replacement the engine actually performed.

use flight_tune::{JournalEvent, RunExecutionContext, TuneError};

use super::test_rig::{
    FakeHandle, SequenceStrategy, TestDirectory, stage, stage_with_execution_retry_limit,
};
use super::{TestTuner, open_stage};

/// Opens a tuner whose Nth simulator start fails execution.
fn open_with_failing_start(
    directory: &TestDirectory,
    state: &FakeHandle,
    limit: u32,
    failing_starts: usize,
) -> Result<TestTuner, TuneError> {
    state.0.borrow_mut().fail_starts_through = failing_starts;
    open_stage(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.5]),
        2.0,
        stage_with_execution_retry_limit(limit),
    )
}

fn retry_events(tuner: &TestTuner) -> Vec<&JournalEvent> {
    tuner
        .journal()
        .entries()
        .iter()
        .map(|entry| &entry.event)
        .filter(|event| {
            matches!(
                event,
                JournalEvent::RetryAuthorized { .. } | JournalEvent::RetryExhausted { .. }
            )
        })
        .collect()
}

fn quarantine_count(tuner: &TestTuner) -> usize {
    tuner
        .journal()
        .entries()
        .iter()
        .filter(|entry| matches!(entry.event, JournalEvent::AttemptQuarantined { .. }))
        .count()
}

fn prepared_contexts(tuner: &TestTuner) -> Vec<&RunExecutionContext> {
    tuner
        .journal()
        .entries()
        .iter()
        .filter_map(|entry| match &entry.event {
            JournalEvent::RunPrepared { context, .. } => Some(context),
            _ => None,
        })
        .collect()
}

#[test]
fn a_quarantined_execution_receives_one_replacement_that_keeps_its_condition() {
    let directory = TestDirectory::new("execution-retry-one-replacement");
    let state = FakeHandle::new();
    let mut tuner =
        open_with_failing_start(&directory, &state, 1, 1).expect("open a retrying tuner");

    tuner
        .run_training_attempts_blocking(0)
        .expect("the replacement settles the training baseline");

    assert_eq!(quarantine_count(&tuner), 1);
    let events = retry_events(&tuner);
    assert_eq!(events.len(), 1, "one quarantine states one decision");
    let JournalEvent::RetryAuthorized {
        source_trial_id,
        replacement_trial_id,
        retry_index,
        ..
    } = events[0]
    else {
        panic!("the declared limit authorizes one replacement");
    };
    assert_eq!(*source_trial_id, 0);
    assert_eq!(*replacement_trial_id, 1);
    assert_eq!(*retry_index, 1);

    // The replacement differs from the execution it replaces in the trial
    // identity and the retry index, and in nothing else.
    let contexts = prepared_contexts(&tuner);
    let source = contexts.first().expect("the first execution");
    let replacement = contexts
        .iter()
        .find(|context| context.retry_index() == 1)
        .expect("the replacement execution");
    assert!(source.states_same_condition(replacement));
    assert_ne!(source.trial_id(), replacement.trial_id());
    assert_eq!(source.retry_index(), 0);
    assert_ne!(
        source.digest().expect("source identity"),
        replacement.digest().expect("replacement identity"),
        "a replacement cannot carry the identity of the execution it replaces"
    );
}

#[test]
fn a_replaced_training_baseline_keeps_one_attempt_index() {
    let directory = TestDirectory::new("execution-retry-attempt-index");
    let state = FakeHandle::new();
    let mut tuner =
        open_with_failing_start(&directory, &state, 1, 1).expect("open a retrying tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("run one challenger after the replacement");

    // The quarantined baseline consumed no challenger index: the replacement
    // stands in its place, so the one challenger that ran is attempt zero.
    assert_eq!(tuner.journal().training_attempt_count(), 1);
    assert!(tuner.journal().entries().iter().any(|entry| matches!(
        entry.event,
        JournalEvent::AttemptPrepared {
            role: flight_tune::AttemptRole::TrainingChallenger { attempt_index: 0 },
            ..
        }
    )));
}

#[test]
fn a_campaign_that_authorizes_no_replacement_stops_at_its_quarantine() {
    let directory = TestDirectory::new("execution-retry-none");
    let state = FakeHandle::new();
    state.0.borrow_mut().fail_starts_through = 1;
    let mut tuner = open_stage(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.5]),
        2.0,
        stage(),
    )
    .expect("open a tuner that authorizes no replacement");

    let error = tuner
        .run_training_attempts_blocking(1)
        .expect_err("a quarantine with no replacement stops the campaign");

    assert!(
        matches!(
            error,
            TuneError::Adapter { .. } | TuneError::InvalidState { .. }
        ),
        "a stopped campaign reports the failure that quarantined it: {error}"
    );
    assert_eq!(quarantine_count(&tuner), 1);
    let events = retry_events(&tuner);
    assert_eq!(events.len(), 1);
    assert!(
        matches!(
            events[0],
            JournalEvent::RetryExhausted {
                source_trial_id: 0,
                retry_index: 0,
                ..
            }
        ),
        "a limit of zero exhausts at the first execution"
    );
}

#[test]
fn replacements_stop_at_the_declared_limit() {
    let directory = TestDirectory::new("execution-retry-limit");
    let state = FakeHandle::new();
    // Both the first execution and its one replacement fail, so the campaign
    // reaches the limit and states exhaustion rather than a third execution.
    let mut tuner =
        open_with_failing_start(&directory, &state, 1, 2).expect("open a retrying tuner");

    let error = tuner
        .run_training_attempts_blocking(1)
        .expect_err("an exhausted retry stops the campaign");

    assert!(
        matches!(
            error,
            TuneError::Adapter { .. } | TuneError::InvalidState { .. }
        ),
        "an exhausted campaign reports the failure that quarantined it: {error}"
    );
    assert_eq!(quarantine_count(&tuner), 2);
    let events = retry_events(&tuner);
    assert_eq!(events.len(), 2, "each quarantine states one decision");
    assert!(matches!(events[0], JournalEvent::RetryAuthorized { .. }));
    assert!(
        matches!(
            events[1],
            JournalEvent::RetryExhausted {
                source_trial_id: 1,
                retry_index: 1,
                ..
            }
        ),
        "the replacement exhausts at the declared limit"
    );
}

#[test]
fn a_quarantine_reason_identity_covers_its_exact_bytes() {
    let directory = TestDirectory::new("execution-retry-reason");
    let state = FakeHandle::new();
    let mut tuner =
        open_with_failing_start(&directory, &state, 1, 1).expect("open a retrying tuner");

    tuner
        .run_training_attempts_blocking(0)
        .expect("the replacement settles the training baseline");

    let reason = tuner
        .journal()
        .entries()
        .iter()
        .find_map(|entry| match &entry.event {
            JournalEvent::AttemptQuarantined { reason, .. } => Some(reason.clone()),
            _ => None,
        })
        .expect("the quarantine reason");
    let saved = retry_events(&tuner)
        .first()
        .and_then(|event| match event {
            JournalEvent::RetryAuthorized {
                quarantine_reason_digest,
                ..
            } => Some(*quarantine_reason_digest),
            _ => None,
        })
        .expect("the authorized retry reason identity");

    assert_eq!(saved, flight_tune::quarantine_reason_digest(&reason));
    assert_ne!(
        saved,
        flight_tune::quarantine_reason_digest(&format!("{reason} ")),
        "one added byte states a different reason"
    );
}

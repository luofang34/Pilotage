use std::fs;
use std::path::Path;

use flight_tune::{AttemptRole, Digest, JournalEntry, JournalEvent, TuneError};
use sha2::{Digest as ShaDigest, Sha256};

use super::test_rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    TestDirectory, candidate, stage,
};
use super::{TestTuner, open};

#[test]
fn recovery_contains_a_pending_run_before_transition_reauthorization() {
    let (directory, state, strategy) = pending_challenger("recovery-before-reauthorization");
    let stop_before = state.0.borrow().stop_count;
    let cleanup_before = state.0.borrow().cleanup_count;
    let lifecycle_before = state.0.borrow().lifecycle.len();
    state.0.borrow_mut().transition.maximum_delta = Some(0.1);

    let result = open(directory.path(), state.clone(), strategy.clone(), 2.0);
    let Err(error) = result else {
        panic!("changed transition behavior resumed the campaign");
    };

    assert!(matches!(error, TuneError::Adapter { .. }));
    assert_eq!(state.0.borrow().stop_count, stop_before.wrapping_add(1));
    assert_eq!(
        state.0.borrow().cleanup_count,
        cleanup_before.wrapping_add(1)
    );
    let lifecycle = state.0.borrow().lifecycle[lifecycle_before..].to_vec();
    let stop_order = lifecycle
        .iter()
        .position(|action| action == "stop")
        .expect("pending stop action");
    let cleanup_order = lifecycle
        .iter()
        .position(|action| action == "cleanup")
        .expect("pending cleanup action");
    let authorization_order = lifecycle
        .iter()
        .position(|action| action == "authorize_transition")
        .expect("transition reauthorization action");
    assert!(stop_order < cleanup_order);
    assert!(cleanup_order < authorization_order);
    state.0.borrow_mut().transition.maximum_delta = None;
    let stop_after_failure = state.0.borrow().stop_count;
    let cleanup_after_failure = state.0.borrow().cleanup_count;
    let resumed =
        open(directory.path(), state.clone(), strategy, 2.0).expect("open contained campaign");
    assert_eq!(state.0.borrow().stop_count, stop_after_failure);
    assert_eq!(state.0.borrow().cleanup_count, cleanup_after_failure);
    let quarantine = resumed
        .journal()
        .entries()
        .iter()
        .position(|entry| matches!(entry.event, JournalEvent::AttemptQuarantined { .. }))
        .expect("quarantine event");
    let cleanup = resumed
        .journal()
        .entries()
        .iter()
        .enumerate()
        .skip(quarantine.wrapping_add(1))
        .find_map(|(index, entry)| {
            matches!(entry.event, JournalEvent::CleanupRecorded { .. }).then_some(index)
        })
        .expect("cleanup event");
    assert!(quarantine < cleanup);
}

#[test]
fn cleanup_failure_prevents_transition_reauthorization() {
    let (directory, state, strategy) = pending_challenger("cleanup-failure-before-reauthorization");
    let lifecycle_before = state.0.borrow().lifecycle.len();
    let authorizations_before = state.0.borrow().transition.authorization_count;
    state.0.borrow_mut().cleanup_fault.return_error();

    let result = open(directory.path(), state.clone(), strategy, 2.0);
    let Err(error) = result else {
        panic!("cleanup failure resumed the campaign");
    };

    assert!(matches!(
        error,
        TuneError::InvalidState {
            operation: "recover pending attempt",
            ..
        }
    ));
    assert_eq!(
        state.0.borrow().transition.authorization_count,
        authorizations_before
    );
    let lifecycle = state.0.borrow().lifecycle[lifecycle_before..].to_vec();
    let stop_order = lifecycle
        .iter()
        .position(|action| action == "stop")
        .expect("pending stop action");
    let cleanup_order = lifecycle
        .iter()
        .position(|action| action == "cleanup")
        .expect("pending cleanup action");
    assert!(stop_order < cleanup_order);
    assert!(
        lifecycle
            .iter()
            .all(|action| action != "authorize_transition")
    );
}

#[test]
fn cleanup_append_loss_repeats_only_idempotent_cleanup() {
    let directory = TestDirectory::new("cleanup-append-loss");
    let state = FakeHandle::new();
    state.0.borrow_mut().timeout_next_sample = true;
    let strategy = SequenceStrategy::new(Vec::new());
    let mut tuner =
        open_tracked(directory.path(), state.clone(), strategy.clone(), 2.0).expect("open tuner");
    assert!(tuner.run_training_attempts_blocking(0).is_err());
    let completed = tuner
        .journal()
        .entries()
        .iter()
        .rev()
        .find(|entry| matches!(entry.event, JournalEvent::AttemptCompleted { .. }))
        .expect("completed hard gate attempt");
    write_head(directory.path(), document_digest(completed));
    let before = CleanupCounts::capture(&state);
    drop(tuner);

    let first = open_tracked(directory.path(), state.clone(), strategy.clone(), 2.0)
        .expect("first cleanup recovery");
    let after_first = CleanupCounts::capture(&state);
    assert_eq!(after_first.gate_cancel, before.gate_cancel.wrapping_add(1));
    assert_eq!(
        after_first.metric_cancel,
        before.metric_cancel.wrapping_add(1)
    );
    assert_eq!(after_first.backend, before.backend.wrapping_add(1));
    drop(first);

    let second = open_tracked(directory.path(), state.clone(), strategy, 2.0)
        .expect("second cleanup recovery");
    assert_eq!(CleanupCounts::capture(&state), after_first);
    drop(second);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CleanupCounts {
    gate_cancel: usize,
    metric_cancel: usize,
    backend: usize,
}

impl CleanupCounts {
    fn capture(state: &FakeHandle) -> Self {
        let state = state.0.borrow();
        Self {
            gate_cancel: state.gate_cancel_count,
            metric_cancel: state.metric_cancel_count,
            backend: state.cleanup_count,
        }
    }
}

#[test]
fn attempt_prepared_challenger_reauthorizes_before_a_new_run() {
    let (directory, state, strategy) =
        attempt_prepared_challenger("attempt-prepared-reauthorization");
    let before = MutationCounts::capture(&state);
    let lifecycle_before = state.0.borrow().lifecycle.len();
    let authorizations_before = state.0.borrow().transition.authorization_count;
    state.0.borrow_mut().transition.maximum_delta = Some(0.1);

    let result = open(directory.path(), state.clone(), strategy, 2.0);
    let Err(error) = result else {
        panic!("changed transition behavior resumed the campaign");
    };

    assert!(matches!(error, TuneError::Adapter { .. }));
    assert_eq!(MutationCounts::capture(&state), before);
    assert_eq!(
        state.0.borrow().transition.authorization_count,
        authorizations_before.wrapping_add(1)
    );
    assert_eq!(
        state.0.borrow().lifecycle[lifecycle_before..],
        ["open_session", "authorize_transition"]
    );
}

#[test]
fn committed_prefix_reauthorizes_before_the_next_run() {
    let directory = TestDirectory::new("committed-prefix-reauthorization");
    let state = FakeHandle::new();
    let strategy = SequenceStrategy::new(vec![0.5]);
    let mut tuner =
        open(directory.path(), state.clone(), strategy.clone(), 2.0).expect("open tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("complete source challenger");
    let trial_id = tuner
        .journal()
        .entries()
        .iter()
        .rev()
        .find_map(|entry| match &entry.event {
            JournalEvent::AttemptPrepared {
                trial_id,
                role: AttemptRole::TrainingChallenger { .. },
                ..
            } => Some(*trial_id),
            _ => None,
        })
        .expect("challenger attempt");
    let first_commit = tuner
        .journal()
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                &entry.event,
                JournalEvent::RunCommitted {
                    trial_id: saved,
                    ..
                } if *saved == trial_id
            )
        })
        .expect("first challenger commit");
    write_head(directory.path(), document_digest(first_commit));
    drop(tuner);

    let before = MutationCounts::capture(&state);
    let cleanup_before = state.0.borrow().cleanup_count;
    let lifecycle_before = state.0.borrow().lifecycle.len();
    state.0.borrow_mut().transition.maximum_delta = Some(0.1);
    let result = open(directory.path(), state.clone(), strategy, 2.0);
    let Err(error) = result else {
        panic!("changed transition behavior resumed the committed prefix");
    };

    assert!(matches!(error, TuneError::Adapter { .. }));
    assert_eq!(MutationCounts::capture(&state), before);
    assert_eq!(
        state.0.borrow().cleanup_count,
        cleanup_before.wrapping_add(1)
    );
    assert_eq!(
        state.0.borrow().lifecycle[lifecycle_before..],
        ["open_session", "cleanup", "authorize_transition"]
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MutationCounts {
    run_bind: usize,
    prepare: usize,
    candidate_ensure: usize,
    candidate_apply: usize,
    gate_begin: usize,
    metric_begin: usize,
    start: usize,
}

impl MutationCounts {
    fn capture(state: &FakeHandle) -> Self {
        let state = state.0.borrow();
        Self {
            run_bind: state.terminal.bind_count(),
            prepare: state.prepare_count,
            candidate_ensure: state.vehicle.ensure_count,
            candidate_apply: state.vehicle.apply_count,
            gate_begin: state.gate_begin_count,
            metric_begin: state.metric_begin_count,
            start: state.start_count,
        }
    }
}

fn pending_challenger(label: &str) -> (TestDirectory, FakeHandle, SequenceStrategy) {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    state.0.borrow_mut().panic_on_prepare = Some(3);
    let strategy = SequenceStrategy::new(vec![0.5]);
    let mut tuner =
        open(directory.path(), state.clone(), strategy.clone(), 2.0).expect("open tuner");
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tuner.run_training_attempts_blocking(1).ok();
    }));
    assert!(stopped.is_err());
    drop(tuner);
    state.0.borrow_mut().panic_on_prepare = None;
    (directory, state, strategy)
}

fn attempt_prepared_challenger(label: &str) -> (TestDirectory, FakeHandle, SequenceStrategy) {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    state.0.borrow_mut().panic_on_prepare = Some(3);
    let strategy = SequenceStrategy::new(vec![0.5]);
    let mut tuner =
        open(directory.path(), state.clone(), strategy.clone(), 2.0).expect("open tuner");
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tuner.run_training_attempts_blocking(1).ok();
    }));
    assert!(stopped.is_err());
    let entry = tuner
        .journal()
        .entries()
        .iter()
        .rev()
        .find(|entry| matches!(entry.event, JournalEvent::AttemptPrepared { .. }))
        .expect("challenger attempt preparation");
    write_head(directory.path(), document_digest(entry));
    drop(tuner);
    state.0.borrow_mut().panic_on_prepare = None;
    (directory, state, strategy)
}

fn document_digest(value: &JournalEntry) -> Digest {
    let bytes = serde_json::to_vec(value).expect("encode journal entry");
    Digest::from_bytes(Sha256::digest(bytes).into())
}

fn write_head(root: &Path, digest: Digest) {
    let document = serde_json::json!({ "digest": digest });
    let bytes = serde_json::to_vec(&document).expect("encode journal head");
    fs::write(root.join("HEAD.json"), bytes).expect("set attempt crash boundary");
}

fn open_tracked(
    path: &Path,
    state: FakeHandle,
    strategy: SequenceStrategy,
    gate_limit: f64,
) -> Result<TestTuner, TuneError> {
    flight_tune::Tuner::open_or_resume(
        path,
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::tracked(gate_limit, state.clone()),
        QuadraticMetric::new(state),
        strategy,
    )
}

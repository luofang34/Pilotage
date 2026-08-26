mod authorization;
mod catalog;
mod evidence;
mod external_action;
mod prospective_authorization;
#[allow(dead_code)]
#[path = "../../../tests/tuner/test_rig.rs"]
mod rig;
mod writer_lease;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pilotage_durable_storage::{
    DurabilityStep, FaultAction, FaultController, FaultRule, StorageError, StorageOperation,
};

use crate::{Journal, JournalEvent, TuneError, Tuner};
use evidence::{
    ActionSnapshot, EvidenceSnapshot, completed_baseline_actions,
    completed_without_final_cleanup_actions,
};
use rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, FakeVehicle, ObservedViews,
    QuadraticMetric, SequenceStrategy, candidate, stage,
};

type TestTuner = Tuner<FakeBackend, FakeVehicle, EnvelopeGates, QuadraticMetric, SequenceStrategy>;

#[test]
fn an_ambiguous_preparation_poison_stops_all_external_action() {
    let directory = TestDirectory::new("prepared-head-poison");
    let faults = head_faults(2);
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_with_faults(&directory, state.clone(), faults.clone());
    let before = ActionSnapshot::new(&state, &proposals);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("poison prepared HEAD");

    assert_ambiguous(error);
    assert!(faults.is_exhausted().expect("read fault state"));
    assert_eq!(ActionSnapshot::new(&state, &proposals), before);
    assert_eq!(tuner.journal().entries().len(), 1);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned_operations(&mut tuner, &directory, &state, &proposals, &poisoned);
    drop(tuner);

    let reopened = reopen_journal(&directory, state);
    assert_eq!(reopened.entries().len(), 2);
    assert!(matches!(
        reopened.entries()[1].event,
        JournalEvent::AttemptPrepared { .. }
    ));
    let pending = reopened.state().pending.as_ref().expect("pending attempt");
    assert!(pending.outcome.is_none());
}

#[test]
fn an_ambiguous_cleanup_poison_skips_candidate_reconciliation() {
    let directory = TestDirectory::new("cleanup-head-poison");
    let faults = head_faults(4);
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_with_faults(&directory, state.clone(), faults.clone());

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("poison cleanup HEAD");

    assert_ambiguous(error);
    assert!(faults.is_exhausted().expect("read fault state"));
    assert_eq!(
        ActionSnapshot::new(&state, &proposals),
        completed_baseline_actions()
    );
    assert_eq!(tuner.journal().entries().len(), 3);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned_operations(&mut tuner, &directory, &state, &proposals, &poisoned);
    drop(tuner);

    let reopened = reopen_journal(&directory, state);
    assert_eq!(reopened.entries().len(), 4);
    assert!(matches!(
        reopened.entries()[3].event,
        JournalEvent::CleanupRecorded { .. }
    ));
    assert!(reopened.state().pending.is_none());
    assert!(reopened.state().training_baseline.is_some());
}

#[test]
fn an_ambiguous_completion_poison_skips_cleanup_and_reconciliation() {
    let directory = TestDirectory::new("completed-head-poison");
    let faults = head_faults(3);
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_with_faults(&directory, state.clone(), faults.clone());

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("poison completed HEAD");

    assert_ambiguous(error);
    assert!(faults.is_exhausted().expect("read fault state"));
    assert_eq!(
        ActionSnapshot::new(&state, &proposals),
        completed_without_final_cleanup_actions()
    );
    assert_eq!(tuner.journal().entries().len(), 2);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned_operations(&mut tuner, &directory, &state, &proposals, &poisoned);
    drop(tuner);

    let reopened = reopen_journal(&directory, state);
    assert_eq!(reopened.entries().len(), 3);
    assert!(matches!(
        reopened.entries()[2].event,
        JournalEvent::AttemptCompleted { .. }
    ));
    let pending = reopened.state().pending.as_ref().expect("pending attempt");
    assert!(pending.outcome.is_some());
}

#[test]
fn an_ambiguous_quarantine_preserves_the_primary_error() {
    let directory = TestDirectory::new("quarantine-head-poison");
    let faults = head_faults(3);
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_with_faults(&directory, state.clone(), faults.clone());
    state.0.borrow_mut().bad_candidate_readback_on_ensure = Some(2);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("poison quarantine HEAD");

    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
    assert!(faults.is_exhausted().expect("read fault state"));
    let actions = state.0.borrow();
    assert_eq!(actions.prepare_count, 1);
    assert_eq!(actions.ensure_count, 2);
    assert_eq!(actions.start_count, 0);
    assert_eq!(actions.stop_count, 0);
    assert_eq!(actions.cleanup_count, 0);
    drop(actions);
    assert_eq!(tuner.journal().entries().len(), 2);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned_operations(&mut tuner, &directory, &state, &proposals, &poisoned);
    drop(tuner);

    let reopened = reopen_journal(&directory, state);
    assert_eq!(reopened.entries().len(), 3);
    assert!(matches!(
        reopened.entries()[2].event,
        JournalEvent::AttemptQuarantined { .. }
    ));
}

#[test]
fn a_stale_head_authorization_poison_stops_all_external_action() {
    let directory = TestDirectory::new("stale-head-poison");
    let state = FakeHandle::new();
    let (mut tuner, proposals) =
        open_with_faults(&directory, state.clone(), FaultController::default());
    let mut conflicting = fs::read(directory.path().join("HEAD.json")).expect("read journal head");
    let digest_tail = conflicting.len().wrapping_sub(3);
    conflicting[digest_tail] = if conflicting[digest_tail] == b'0' {
        b'1'
    } else {
        b'0'
    };
    fs::write(directory.path().join("HEAD.json"), conflicting).expect("replace journal head");
    let before = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject stale HEAD");

    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::ContentMismatch { context }
                if context.object.is_some())
    ));
    assert_snapshot(&tuner, &directory, &state, &proposals, &before);
    assert_eq!(tuner.journal().entries().len(), 1);
    assert_poisoned_operations(&mut tuner, &directory, &state, &proposals, &before);
}

#[test]
fn a_failed_authorization_rename_leaves_an_inert_orphan() {
    let directory = TestDirectory::new("inert-orphan");
    let faults = FaultController::new([FaultRule::on_occurrence(
        StorageOperation::CompareExchange,
        DurabilityStep::AuthorizationRename,
        2,
        FaultAction::FailBefore,
    )]);
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_with_faults(&directory, state.clone(), faults.clone());
    let head_before = fs::read(directory.path().join("HEAD.json")).expect("read journal head");
    let actions_before = ActionSnapshot::new(&state, &proposals);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("fail before HEAD rename");

    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::InjectedFault { .. })
    ));
    assert!(faults.is_exhausted().expect("read fault state"));
    assert_eq!(ActionSnapshot::new(&state, &proposals), actions_before);
    assert_eq!(tuner.journal().entries().len(), 1);
    assert_eq!(
        fs::read(directory.path().join("HEAD.json")).expect("read unchanged journal head"),
        head_before
    );
    assert_eq!(
        fs::read_dir(directory.path().join("entries"))
            .expect("read entry directory")
            .count(),
        2
    );
    drop(tuner);

    let reopened = reopen_journal(&directory, state);
    assert_eq!(reopened.entries().len(), 1);
    assert!(matches!(
        reopened.entries()[0].event,
        JournalEvent::Started { .. }
    ));
}

fn open_with_faults(
    directory: &TestDirectory,
    state: FakeHandle,
    faults: FaultController,
) -> (TestTuner, ObservedViews) {
    let strategy = SequenceStrategy::new(Vec::new());
    let proposals = strategy.views.clone();
    let tuner = Tuner::open_or_resume_with_faults(
        directory.path(),
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::tracked(2.0, state.clone()),
        QuadraticMetric::new(state),
        strategy,
        faults,
    )
    .expect("open fault-enabled tuner");
    (tuner, proposals)
}

fn reopen_journal(directory: &TestDirectory, state: FakeHandle) -> Journal {
    let backend = FakeBackend::new(state.clone());
    let factory = FakeFactory::new(state.clone());
    let gates = EnvelopeGates::tracked(2.0, state.clone());
    let metric = QuadraticMetric::new(state);
    let strategy = SequenceStrategy::new(Vec::new());
    let runtimes =
        super::super::validate_open_components(&backend, &factory, &gates, &metric, &strategy)
            .expect("validate runtime identities");
    Journal::open_or_create(directory.path(), &stage(), 91, runtimes, &candidate(0.0))
        .expect("reopen durable journal")
}

fn head_faults(parent_occurrence: u64) -> FaultController {
    FaultController::new([
        FaultRule::on_occurrence(
            StorageOperation::CompareExchange,
            DurabilityStep::ParentDirectory,
            parent_occurrence,
            FaultAction::LoseAckAfter,
        ),
        FaultRule::once(
            StorageOperation::CompareExchange,
            DurabilityStep::RecoveryBarrier,
            FaultAction::LoseAckAfter,
        ),
    ])
}

fn assert_ambiguous(error: TuneError) {
    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::AmbiguousCommit { .. })
    ));
}

fn assert_poisoned_operations(
    tuner: &mut TestTuner,
    directory: &TestDirectory,
    state: &FakeHandle,
    proposals: &ObservedViews,
    expected: &EvidenceSnapshot,
) {
    assert_poisoned(tuner.run_training_attempts_blocking(0));
    assert_snapshot(tuner, directory, state, proposals, expected);
    assert_poisoned(tuner.freeze_candidate());
    assert_snapshot(tuner, directory, state, proposals, expected);
    assert_poisoned(tuner.run_promotion_once_blocking());
    assert_snapshot(tuner, directory, state, proposals, expected);
    assert_poisoned(tuner.run_final_qualification_once_blocking());
    assert_snapshot(tuner, directory, state, proposals, expected);
    assert_poisoned(tuner.qualified_candidate());
    assert_snapshot(tuner, directory, state, proposals, expected);
}

fn assert_poisoned<T>(result: Result<T, TuneError>) {
    assert!(matches!(result, Err(TuneError::JournalPoisoned)));
}

fn assert_snapshot(
    tuner: &TestTuner,
    directory: &TestDirectory,
    state: &FakeHandle,
    proposals: &ObservedViews,
    expected: &EvidenceSnapshot,
) {
    assert_eq!(
        &EvidenceSnapshot::new(tuner, directory, state, proposals),
        expected
    );
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = test_root().join(format!("flight-tune-{label}-{}-{time}", std::process::id()));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn test_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/private/tmp")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::temp_dir()
            .canonicalize()
            .expect("canonical test temporary directory")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).ok();
    }
}

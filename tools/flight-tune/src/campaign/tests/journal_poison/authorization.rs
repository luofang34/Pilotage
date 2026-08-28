use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

use pilotage_durable_storage::FaultController;
#[cfg(unix)]
use pilotage_durable_storage::StorageError;

use super::rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    candidate, stage,
};
use super::{EvidenceSnapshot, TestDirectory, TestTuner, assert_poisoned, assert_snapshot};
use crate::{FinalQualificationOutcome, TuneError, Tuner};

#[test]
fn a_changed_head_stops_pending_outcome_recovery_before_external_action() {
    let directory = TestDirectory::new("pending-outcome-head-change");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), Vec::new());
    state.0.borrow_mut().cleanup_fault.panic_on(2);
    let stopped = catch_unwind(AssertUnwindSafe(|| {
        tuner.run_training_attempts_blocking(0).ok();
    }));
    assert!(stopped.is_err());
    state.0.borrow_mut().cleanup_fault.clear();
    let pending = tuner
        .journal()
        .state()
        .pending
        .as_ref()
        .expect("pending outcome");
    assert!(pending.outcome.is_some());
    change_head_digest(directory.path());
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject changed HEAD before recovery");

    assert_changed_head(error);
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.qualified_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
}

#[test]
fn a_changed_head_stops_a_qualified_candidate_read() {
    let directory = TestDirectory::new("qualified-read-head-change");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), vec![0.5]);
    seal_qualified(&mut tuner);
    change_head_digest(directory.path());
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .qualified_candidate()
        .expect_err("reject qualified read with changed HEAD");

    assert_changed_head(error);
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.run_final_qualification_once_blocking());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
}

#[test]
fn a_changed_head_stops_saved_result_reconciliation() {
    let directory = TestDirectory::new("saved-result-head-change");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), vec![0.5]);
    seal_qualified(&mut tuner);
    change_head_digest(directory.path());
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .run_final_qualification_once_blocking()
        .expect_err("reject saved result reconciliation with changed HEAD");

    assert_changed_head(error);
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.qualified_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
}

#[test]
#[cfg(unix)]
fn a_replaced_root_poison_stops_the_first_public_action() {
    let directory = TestDirectory::new("replaced-root");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone(), Vec::new());
    let swap = RootSwap::install(directory.path());
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject replaced journal root");

    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::RootChanged { .. })
    ));
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.freeze_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    drop(tuner);
    drop(swap);
}

fn open_tuner(
    directory: &TestDirectory,
    state: FakeHandle,
    proposals: Vec<f64>,
) -> (TestTuner, super::rig::ObservedViews) {
    let strategy = SequenceStrategy::new(proposals);
    let views = strategy.views.clone();
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
        FaultController::default(),
    )
    .expect("open tuner");
    (tuner, views)
}

fn seal_qualified(tuner: &mut TestTuner) {
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");
    tuner.run_promotion_once_blocking().expect("run promotion");
    assert_eq!(
        tuner
            .run_final_qualification_once_blocking()
            .expect("run qualification"),
        FinalQualificationOutcome::Qualified
    );
}

fn assert_changed_head(error: TuneError) {
    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::ContentMismatch { context }
                if context.object.is_some())
    ));
}

fn change_head_digest(root: &Path) {
    let head = root.join("HEAD.json");
    let mut bytes = fs::read(&head).expect("read journal head");
    let digest_tail = bytes.len().checked_sub(3).expect("HEAD digest byte");
    bytes[digest_tail] = if bytes[digest_tail] == b'0' {
        b'1'
    } else {
        b'0'
    };
    fs::write(head, bytes).expect("change journal head");
}

#[cfg(unix)]
struct RootSwap {
    root: PathBuf,
    held: PathBuf,
}

#[cfg(unix)]
impl RootSwap {
    fn install(root: &Path) -> Self {
        let held = root.with_extension("held");
        fs::rename(root, &held).expect("hold anchored journal root");
        symlink(&held, root).expect("replace journal root with symlink");
        Self {
            root: root.to_path_buf(),
            held,
        }
    }
}

#[cfg(unix)]
impl Drop for RootSwap {
    fn drop(&mut self) {
        fs::remove_file(&self.root).expect("remove replacement symlink");
        fs::rename(&self.held, &self.root).expect("restore anchored journal root");
    }
}

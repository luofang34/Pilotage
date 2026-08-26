use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use pilotage_durable_storage::{DurableStore, FaultController, StorageError, WriterLease};

use super::rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    candidate, stage,
};
use super::{EvidenceSnapshot, TestDirectory, TestTuner, assert_poisoned, assert_snapshot};
use crate::{FinalQualificationOutcome, TuneError, Tuner};

#[test]
fn a_replaced_writer_lock_stops_saved_result_reconciliation() {
    let directory = TestDirectory::new("replaced-writer-lock");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone());
    seal_qualified(&mut tuner);
    let mut swap = WriterLockSwap::install(directory.path());
    let expected = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);

    let error = tuner
        .run_final_qualification_once_blocking()
        .expect_err("reject replaced writer lock");

    assert_writer_binding_changed(error);
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    assert_poisoned(tuner.qualified_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &expected);
    drop(tuner);
    swap.restore();
}

#[test]
fn a_writer_lock_change_after_the_final_catalog_scan_fails_that_audit() {
    let directory = TestDirectory::new("final-scan-writer-lock-change");
    let state = FakeHandle::new();
    let (tuner, proposals) = open_tuner(&directory, state.clone());
    let mut swap = None;

    let error = tuner
        .journal()
        .ensure_usable_with_final_hook_for_test(|| {
            swap = Some(WriterLockSwap::install(directory.path()));
        })
        .expect_err("reject writer lock changed after the catalog scan");

    assert_writer_binding_changed(error);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned(tuner.qualified_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &poisoned);
    drop(tuner);
    swap.as_mut().expect("installed writer swap").restore();
}

fn open_tuner(
    directory: &TestDirectory,
    state: FakeHandle,
) -> (TestTuner, super::rig::ObservedViews) {
    let strategy = SequenceStrategy::new(vec![0.5]);
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

fn assert_writer_binding_changed(error: TuneError) {
    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::Corruption { reason, .. }
                if *reason == "writer lease name binding changed")
    ));
}

struct WriterLockSwap {
    original: PathBuf,
    held: PathBuf,
    second_lease: Option<WriterLease>,
    second_store: Option<DurableStore>,
    restored: bool,
}

impl WriterLockSwap {
    fn install(root: &Path) -> Self {
        let original = root.join(".pilotage-writer-lock");
        let held = root.with_extension("held-writer-lock");
        fs::rename(&original, &held).expect("hold writer lock outside the journal root");
        let replacement = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&original)
            .expect("create replacement writer lock");
        replacement.sync_all().expect("sync replacement lock");
        drop(replacement);
        let second_store = DurableStore::open_or_create(root).expect("open replacement store");
        let second_lease = second_store
            .acquire_writer()
            .expect("acquire replacement writer lock");
        Self {
            original,
            held,
            second_lease: Some(second_lease),
            second_store: Some(second_store),
            restored: false,
        }
    }

    fn restore(&mut self) {
        drop(self.second_lease.take());
        drop(self.second_store.take());
        fs::remove_file(&self.original).expect("remove replacement writer lock");
        fs::rename(&self.held, &self.original).expect("restore held writer lock");
        self.restored = true;
    }
}

impl Drop for WriterLockSwap {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        drop(self.second_lease.take());
        drop(self.second_store.take());
        fs::remove_file(&self.original).ok();
        fs::rename(&self.held, &self.original).ok();
    }
}

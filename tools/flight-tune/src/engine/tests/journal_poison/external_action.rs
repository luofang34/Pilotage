use pilotage_durable_storage::{FaultController, StorageError};

use super::rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    candidate, stage,
};
use super::{EvidenceSnapshot, TestDirectory, TestTuner, assert_poisoned, assert_snapshot};
use crate::{TuneError, Tuner};

#[test]
fn a_head_change_during_prepare_stops_the_next_external_action() {
    let directory = TestDirectory::new("prepare-head-change");
    let state = FakeHandle::new();
    let (mut tuner, proposals) = open_tuner(&directory, state.clone());
    let before = action_counts(&state);
    state.0.borrow_mut().change_head_on_prepare = Some(directory.path().to_path_buf());

    let error = tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject HEAD change after prepare");

    assert_changed_head(error);
    let after = action_counts(&state);
    assert_eq!(after.prepare, before.prepare.wrapping_add(1));
    assert_eq!(after.ensure, before.ensure);
    assert_eq!(after.gate_begin, before.gate_begin);
    assert_eq!(after.metric_begin, before.metric_begin);
    assert_eq!(after.start, before.start);
    assert_eq!(after.stop, before.stop);
    assert_eq!(after.cleanup, before.cleanup);
    let poisoned = EvidenceSnapshot::new(&tuner, &directory, &state, &proposals);
    assert_poisoned(tuner.freeze_candidate());
    assert_snapshot(&tuner, &directory, &state, &proposals, &poisoned);
}

fn open_tuner(
    directory: &TestDirectory,
    state: FakeHandle,
) -> (TestTuner, super::rig::ObservedViews) {
    let strategy = SequenceStrategy::new(Vec::new());
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

#[derive(Clone, Copy)]
struct ActionCounts {
    prepare: usize,
    ensure: usize,
    gate_begin: usize,
    metric_begin: usize,
    start: usize,
    stop: usize,
    cleanup: usize,
}

fn action_counts(state: &FakeHandle) -> ActionCounts {
    let state = state.0.borrow();
    ActionCounts {
        prepare: state.prepare_count,
        ensure: state.ensure_count,
        gate_begin: state.gate_begin_count,
        metric_begin: state.metric_begin_count,
        start: state.start_count,
        stop: state.stop_count,
        cleanup: state.cleanup_count,
    }
}

fn assert_changed_head(error: TuneError) {
    assert!(matches!(
        error,
        TuneError::Storage { source }
            if matches!(source.as_ref(), StorageError::ContentMismatch { context }
                if context.object.is_some())
    ));
}

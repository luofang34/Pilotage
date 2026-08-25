use flight_tune::PromotionDecision;

use super::TestDirectory;
use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy, candidate};

#[test]
fn hard_gate_rejection_restores_the_training_incumbent() {
    let directory = TestDirectory::new("hard-gate-restores-incumbent");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![1.5]),
        1.0,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("evaluate rejected challenger");

    assert_eq!(state.0.borrow().vehicle.gain, 0.0);
    assert_eq!(state.0.borrow().vehicle.apply_count, 3);
    assert_eq!(
        tuner
            .journal()
            .training_incumbent()
            .expect("read incumbent"),
        candidate(0.0)
    );
}

#[test]
fn passing_but_worse_challenger_restores_the_later_incumbent() {
    let directory = TestDirectory::new("worse-restores-later-incumbent");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![1.0, 0.5]),
        2.0,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(2)
        .expect("evaluate two challengers");

    assert_eq!(state.0.borrow().vehicle.gain, 1.0);
    assert_eq!(state.0.borrow().vehicle.apply_count, 4);
    assert_eq!(
        tuner
            .journal()
            .training_incumbent()
            .expect("read incumbent"),
        candidate(1.0)
    );
}

#[test]
fn selected_challenger_does_not_get_a_second_controller_write() {
    let directory = TestDirectory::new("selected-candidate-idempotent");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.5]),
        2.0,
    )
    .expect("open tuner");

    tuner
        .run_training_attempts_blocking(1)
        .expect("select challenger");

    assert_eq!(state.0.borrow().vehicle.gain, 0.5);
    assert_eq!(state.0.borrow().vehicle.apply_count, 2);
}

#[test]
fn rejected_promotion_restores_the_initial_release_candidate() {
    let directory = TestDirectory::new("promotion-restores-release");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![0.1]),
        2.0,
    )
    .expect("open tuner");
    tuner
        .run_training_attempts_blocking(1)
        .expect("run training");
    tuner.freeze_candidate().expect("freeze candidate");

    assert!(matches!(
        tuner.run_promotion_once_blocking().expect("run promotion"),
        PromotionDecision::RejectedNoImprovement { .. }
    ));
    assert_eq!(state.0.borrow().vehicle.gain, 0.0);
}

#[test]
fn reconciliation_readback_failure_stops_before_the_next_candidate() {
    let directory = TestDirectory::new("reconciliation-readback-stops-search");
    let state = FakeHandle::new();
    state.0.borrow_mut().vehicle.bad_candidate_readback_on_apply = Some(3);
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(vec![1.5, 0.5]),
        1.0,
    )
    .expect("open tuner");

    let error = tuner
        .run_training_attempts_blocking(2)
        .expect_err("reject reconciliation readback");

    assert!(matches!(
        error,
        flight_tune::TuneError::ReceiptMismatch { .. }
    ));
    assert_eq!(tuner.journal().training_attempt_count(), 1);
    assert_eq!(state.0.borrow().prepare_count, 3);
    assert_eq!(state.0.borrow().start_count, 3);
}

#[test]
fn restart_restores_once_after_candidate_activation() {
    let directory = TestDirectory::new("restart-restores-once");
    let state = FakeHandle::new();
    state.0.borrow_mut().panic_on_start = Some(3);
    let strategy = SequenceStrategy::new(vec![0.5]);
    let mut tuner =
        open(directory.path(), state.clone(), strategy.clone(), 2.0).expect("open tuner");

    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tuner.run_training_attempts_blocking(1).ok();
    }));
    assert!(stopped.is_err());
    assert_eq!(state.0.borrow().vehicle.gain, 0.5);
    assert_eq!(state.0.borrow().vehicle.apply_count, 2);
    drop(tuner);
    state.0.borrow_mut().panic_on_start = None;

    let resumed =
        open(directory.path(), state.clone(), strategy.clone(), 2.0).expect("resume tuner");
    assert_eq!(state.0.borrow().vehicle.gain, 0.0);
    assert_eq!(state.0.borrow().vehicle.apply_count, 3);
    drop(resumed);

    let reopened = open(directory.path(), state.clone(), strategy, 2.0).expect("reopen tuner");
    assert_eq!(state.0.borrow().vehicle.gain, 0.0);
    assert_eq!(state.0.borrow().vehicle.apply_count, 3);
    drop(reopened);
}

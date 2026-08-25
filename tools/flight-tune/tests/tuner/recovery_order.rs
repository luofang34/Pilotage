use flight_tune::{JournalEvent, TuneError};

use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy, TestDirectory};

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

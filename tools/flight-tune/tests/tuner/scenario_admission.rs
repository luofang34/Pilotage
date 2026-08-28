use flight_tune::TuneError;

use super::TestDirectory;
use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy};

#[test]
fn changed_mission_content_fails_before_run_mutation() {
    assert_admission_refusal("changed-mission-content", |state| {
        state.0.borrow_mut().bad_mission_content = true;
    });
}

#[test]
fn a_foreign_mission_revision_fails_before_run_mutation() {
    assert_admission_refusal("foreign-mission-revision", |state| {
        state.0.borrow_mut().bad_mission_revision = true;
    });
}

fn assert_admission_refusal(label: &str, tamper: impl FnOnce(&FakeHandle)) {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");
    tamper(&state);

    let result = tuner.run_training_attempts_blocking(0);

    assert!(matches!(result, Err(TuneError::ReceiptMismatch { .. })));
    let state = state.0.borrow();
    assert_eq!(state.prepare_count, 0);
    assert_eq!(state.start_count, 0);
    assert_eq!(state.scenario_action_stop_count, 0);
    assert_eq!(state.scenario_action_cleanup_count, 0);
}

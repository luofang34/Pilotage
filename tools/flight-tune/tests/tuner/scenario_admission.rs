use flight_tune::TuneError;

use super::TestDirectory;
use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy};

#[test]
fn changed_scenario_content_fails_before_run_mutation() {
    let directory = TestDirectory::new("changed-scenario-content");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");
    state.0.borrow_mut().bad_scenario_document = true;

    let result = tuner.run_training_attempts_blocking(0);

    assert!(matches!(result, Err(TuneError::ReceiptMismatch { .. })));
    let state = state.0.borrow();
    assert_eq!(state.prepare_count, 0);
    assert_eq!(state.start_count, 0);
    assert_eq!(state.scenario_action_stop_count, 0);
    assert_eq!(state.scenario_action_cleanup_count, 0);
}

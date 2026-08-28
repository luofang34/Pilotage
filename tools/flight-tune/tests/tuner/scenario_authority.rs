use super::TestDirectory;
use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy};

#[test]
fn authority_change_during_sampling_precedes_vehicle_action() {
    let directory = TestDirectory::new("sample-action-authority");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");
    state.0.borrow_mut().change_head_on_sample = Some(directory.path().to_owned());

    tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject changed authority");

    let state = state.0.borrow();
    assert_eq!(state.scenario_action_observe_count, 0);
    assert_eq!(state.scenario_action_stop_count, 1);
    assert_eq!(state.scenario_action_cleanup_count, 1);
}

#[test]
fn authority_change_during_action_prepare_precedes_action_start() {
    let directory = TestDirectory::new("prepared-action-authority");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");
    state.0.borrow_mut().change_head_on_action_prepare = Some(directory.path().to_owned());

    tuner
        .run_training_attempts_blocking(0)
        .expect_err("reject changed authority");

    let state = state.0.borrow();
    assert_eq!(state.scenario_action_start_count, 0);
    assert_eq!(state.scenario_action_observe_count, 0);
    assert_eq!(state.scenario_action_stop_count, 1);
    assert_eq!(state.scenario_action_cleanup_count, 1);
}

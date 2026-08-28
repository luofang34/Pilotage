use flight_tune::{CandidateEvaluation, JournalEvent, TuneError};

use super::TestDirectory;
use super::open;
use super::test_rig::{FakeHandle, SequenceStrategy};

#[test]
fn completion_without_samples_is_a_core_hard_gate_failure() {
    let directory = TestDirectory::new("completion-without-samples");
    let state = FakeHandle::new();
    let mut tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");
    state.0.borrow_mut().complete_without_sample = true;

    assert!(matches!(
        tuner.run_training_attempts_blocking(0),
        Err(TuneError::UnsafeBaseline { .. })
    ));
    assert_eq!(state.0.borrow().metric_observe_count, 0);
    assert_eq!(state.0.borrow().scenario_action_stop_count, 1);
    assert_eq!(state.0.borrow().scenario_action_cleanup_count, 1);
    assert!(tuner.journal().entries().iter().any(|entry| {
        matches!(
            &entry.event,
            JournalEvent::AttemptCompleted {
                evaluation: CandidateEvaluation::HardGateFailed { failure, .. },
                ..
            } if failure.gate.id == "core.no_samples"
        )
    }));
}

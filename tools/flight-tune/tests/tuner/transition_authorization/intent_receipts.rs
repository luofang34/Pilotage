use flight_tune::{
    AttemptRole, JournalEntry, JournalEvent, RunExecutionContext, RunTerminalOperation, TuneError,
};

use crate::test_rig::{FakeFactory, FakeHandle, SequenceStrategy, TestDirectory};

use super::{ExternalMutations, open};

#[test]
fn preparation_intent_mismatch_stops_vehicle_and_start_mutation() {
    assert_intent_mismatch(IntentFault::Preparation);
}

#[test]
fn vehicle_intent_mismatch_stops_start_mutation() {
    assert_intent_mismatch(IntentFault::Vehicle);
}

#[test]
fn start_intent_mismatch_stops_sampling() {
    assert_intent_mismatch(IntentFault::Start);
}

#[derive(Clone, Copy)]
enum IntentFault {
    Preparation,
    Vehicle,
    Start,
}

fn assert_intent_mismatch(fault: IntentFault) {
    let directory = TestDirectory::new("run-intent-mismatch");
    let state = FakeHandle::new();
    let mut tuner = open(
        &directory,
        state.clone(),
        FakeFactory::new(state.clone()),
        SequenceStrategy::new(vec![0.5]),
    )
    .expect("open tuner");
    tuner
        .run_training_attempts_blocking(0)
        .expect("run safe baseline");
    let before = ExternalMutations::capture(&state);
    let terminal_order_before = state.0.borrow().terminal.operation_order().len();
    set_fault(&state, fault);

    let error = tuner
        .run_training_attempts_blocking(1)
        .expect_err("reject mismatched run intent receipt");

    assert!(matches!(error, TuneError::ReceiptMismatch { .. }));
    assert_containment(&state, before, terminal_order_before, fault);
    assert_quarantine_before_cleanup(tuner.journal().entries());
}

fn set_fault(state: &FakeHandle, fault: IntentFault) {
    let mut state = state.0.borrow_mut();
    match fault {
        IntentFault::Preparation => state.transition.bad_preparation_intent = true,
        IntentFault::Vehicle => state.transition.bad_vehicle_intent = true,
        IntentFault::Start => state.transition.bad_start_intent = true,
    }
}

fn assert_containment(
    state: &FakeHandle,
    before: ExternalMutations,
    terminal_order_before: usize,
    fault: IntentFault,
) {
    let observed = state.0.borrow();
    assert_eq!(observed.sample_poll_count, before.sample_poll);
    assert_eq!(observed.cleanup_count, before.cleanup.wrapping_add(1));
    let prepared = challenger_contexts(&observed.transition.prepared_contexts);
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].transition_authorization().is_some());
    assert_fault_boundary(&observed.transition, fault);
    assert_eq!(observed.stop_count, before.stop.wrapping_add(1));
    assert_eq!(
        &observed.terminal.operation_order()[terminal_order_before..],
        &[
            RunTerminalOperation::ControlStop,
            RunTerminalOperation::TraceStop,
            RunTerminalOperation::ChildHealth,
            RunTerminalOperation::TraceShutdown,
            RunTerminalOperation::ChildTerminate,
        ]
    );
}

fn assert_fault_boundary(state: &crate::test_rig::FakeTransitionState, fault: IntentFault) {
    match fault {
        IntentFault::Preparation => {
            assert!(challenger_contexts(&state.vehicle_contexts).is_empty());
            assert!(challenger_contexts(&state.started_contexts).is_empty());
        }
        IntentFault::Vehicle => assert!(challenger_contexts(&state.started_contexts).is_empty()),
        IntentFault::Start => assert_eq!(challenger_contexts(&state.started_contexts).len(), 1),
    }
}

fn challenger_contexts(contexts: &[RunExecutionContext]) -> Vec<RunExecutionContext> {
    contexts
        .iter()
        .filter(|context| matches!(context.role(), AttemptRole::TrainingChallenger { .. }))
        .cloned()
        .collect()
}

fn assert_quarantine_before_cleanup(entries: &[JournalEntry]) {
    let quarantine = entries
        .iter()
        .position(|entry| matches!(entry.event, JournalEvent::AttemptQuarantined { .. }))
        .expect("quarantine event");
    let cleanup = entries
        .iter()
        .enumerate()
        .skip(quarantine.wrapping_add(1))
        .find_map(|(index, entry)| {
            matches!(entry.event, JournalEvent::CleanupRecorded { .. }).then_some(index)
        })
        .expect("cleanup event");
    assert!(quarantine < cleanup);
}

use super::CommandOutcome;
use crate::wire;

const ALL: [CommandOutcome; 7] = [
    CommandOutcome::Accepted,
    CommandOutcome::InProgress,
    CommandOutcome::TemporarilyRejected,
    CommandOutcome::Denied,
    CommandOutcome::Unsupported,
    CommandOutcome::Failed,
    CommandOutcome::Cancelled,
];

#[test]
fn every_outcome_round_trips_through_the_wire() {
    for outcome in ALL {
        let raw = outcome.to_wire() as i32;
        assert_eq!(CommandOutcome::from_wire(raw), Some(outcome), "{outcome:?}");
    }
}

#[test]
fn unspecified_and_unknown_values_are_no_answer() {
    assert_eq!(
        CommandOutcome::from_wire(wire::CommandOutcome::Unspecified as i32),
        None
    );
    assert_eq!(CommandOutcome::from_wire(99), None);
    assert_eq!(CommandOutcome::from_wire(-1), None);
}

#[test]
fn accepted_is_never_the_wire_default() {
    // A host that never fills the field must not read as accepting.
    assert_ne!(CommandOutcome::Accepted.to_wire() as i32, 0);
}

#[test]
fn only_accepted_and_in_progress_are_not_refusals() {
    let not_refusals: Vec<_> = ALL.into_iter().filter(|o| !o.is_refusal()).collect();
    assert_eq!(
        not_refusals,
        [CommandOutcome::Accepted, CommandOutcome::InProgress]
    );
}

use super::command_outcome;
use pilotage_adapter_api::CommandOutcome;

#[test]
fn each_mav_result_maps_to_its_typed_outcome() {
    let table = [
        (0, CommandOutcome::Accepted),
        (1, CommandOutcome::TemporarilyRejected),
        (2, CommandOutcome::Denied),
        (3, CommandOutcome::Unsupported),
        (4, CommandOutcome::Failed),
        (5, CommandOutcome::InProgress),
        (6, CommandOutcome::Cancelled),
        (7, CommandOutcome::Unsupported),
        (8, CommandOutcome::Unsupported),
        (9, CommandOutcome::Unsupported),
        (10, CommandOutcome::Denied),
    ];
    for (code, outcome) in table {
        assert_eq!(command_outcome(code), outcome, "MAV_RESULT {code}");
    }
}

#[test]
fn an_unknown_code_is_a_refusal_not_an_acceptance() {
    for code in [11, 100, u8::MAX] {
        assert!(command_outcome(code).is_refusal(), "MAV_RESULT {code}");
    }
}

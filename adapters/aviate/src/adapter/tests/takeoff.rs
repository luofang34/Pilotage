use pilotage_protocol::ControlAction;

#[test]
fn native_flight_procedures_are_rejected_without_invoking_the_arm_path() {
    let mut arm_calls = 0_u32;
    let results = super::super::process_flight_actions(
        &[ControlAction::Takeoff, ControlAction::Land],
        |_| arm_calls = arm_calls.wrapping_add(1),
        0.0,
    );
    assert_eq!(arm_calls, 0);
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0],
        pilotage_adapter_api::ActionResult::rejected(
            ControlAction::Takeoff,
            "not supported on the flight scope",
        )
    );
    assert_eq!(
        results[1],
        pilotage_adapter_api::ActionResult::rejected(
            ControlAction::Land,
            "not supported on the flight scope",
        )
    );
}

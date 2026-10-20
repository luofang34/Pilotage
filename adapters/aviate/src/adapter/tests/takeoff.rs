use pilotage_protocol::ControlAction;

#[test]
fn takeoff_is_rejected_without_invoking_the_arm_path() {
    let mut arm_calls = 0_u32;
    let results = super::super::process_flight_actions(
        &[ControlAction::Takeoff],
        |_| arm_calls = arm_calls.wrapping_add(1),
        0.0,
    );
    assert_eq!(arm_calls, 0);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0],
        pilotage_adapter_api::ActionResult::rejected(
            ControlAction::Takeoff,
            "not supported on the flight scope",
        )
    );
}

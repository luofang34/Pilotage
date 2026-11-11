use pilotage_protocol::wire;

use super::FlightWatch;

fn report(arm_state: u32, landed: wire::LandedState) -> wire::FcState {
    wire::FcState {
        arm_state,
        stamp: Some(wire::MeasurementStamp::default()),
        landed_state: Some(wire::LandedStateObservation {
            state: landed as i32,
            stamp: Some(wire::MeasurementStamp::default()),
        }),
        ..wire::FcState::default()
    }
}

#[test]
fn a_landing_after_flight_or_a_disarm_after_an_arm_ends_the_run() {
    let mut watch = FlightWatch::default();
    assert_eq!(watch.observe(&report(1, wire::LandedState::OnGround)), None);
    assert_eq!(
        watch.observe(&report(2, wire::LandedState::TakingOff)),
        None
    );
    assert_eq!(watch.observe(&report(2, wire::LandedState::InAir)), None);
    assert_eq!(
        watch.observe(&report(2, wire::LandedState::Landing)),
        Some("the vehicle landed outside the plan")
    );
    let mut watch = FlightWatch::default();
    assert_eq!(watch.observe(&report(2, wire::LandedState::OnGround)), None);
    assert_eq!(
        watch.observe(&report(1, wire::LandedState::OnGround)),
        Some("the vehicle disarmed outside the plan")
    );
}

#[test]
fn an_unstamped_report_says_nothing() {
    let mut watch = FlightWatch::default();
    let mut unstamped = report(2, wire::LandedState::InAir);
    unstamped.stamp = None;
    if let Some(landed) = unstamped.landed_state.as_mut() {
        landed.stamp = None;
    }
    assert_eq!(watch.observe(&unstamped), None);
    assert_eq!(watch.observe(&report(1, wire::LandedState::OnGround)), None);
}

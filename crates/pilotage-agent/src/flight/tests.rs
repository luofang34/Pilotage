#![allow(clippy::expect_used, clippy::panic)]

use super::{AgentFlight, VehicleOffer};
use crate::directive::{Arrival, Directive};
use crate::executor::Phase;
use crate::model_port::{ModelReply, Refusal};
use crate::scenario::Scenario;
use crate::state::VehicleState;

const CHART: &str = r#"{
  "id": "flight-test",
  "fixes": {"ALPHA": {"north_m": 15.0, "east_m": 0.0}},
  "cruise_height_m": 5.0,
  "arrival_radius_m": 1.5,
  "max_range_m": 35.0,
  "height_m": {"min": 2.0, "max": 30.0},
  "expect": {"end": {"state": "checkpoints_only"}, "timeout_s": 60.0}
}"#;

fn flight() -> AgentFlight {
    let scenario = Scenario::parse(CHART).expect("the test chart is valid");
    AgentFlight::new(
        &scenario,
        VehicleOffer {
            max_linear_mps: 5.0,
            disarm_offered: true,
        },
    )
}

fn reply(directive: Directive) -> ModelReply {
    ModelReply {
        id: 0,
        directive,
        probabilities: Default::default(),
        model_ms: 1.0,
    }
}

fn on_the_pad() -> VehicleState {
    VehicleState {
        north_m: 0.0,
        east_m: 0.0,
        vel_north_mps: 0.0,
        vel_east_mps: 0.0,
        height_m: Some(0.0),
        climb_mps: 0.0,
        yaw_rad: 0.0,
        armed: Some(false),
    }
}

fn direct_to(fix: &str) -> Directive {
    Directive::DirectTo {
        fix: fix.to_owned(),
        on_arrival: Arrival::Hold,
    }
}

#[test]
fn a_request_carries_the_message_the_envelope_and_the_legend() {
    let request = flight().request("Proceed direct ALPHA.", Vec::new());
    assert_eq!(request.message, "Proceed direct ALPHA.");
    assert!(request.envelope.fixes.contains(&"ALPHA".to_owned()));
    assert!(request.legend.contains("ALPHA"));
    assert_eq!(request.envelope.speed_mps.max, 5.0);
    assert_eq!(request.envelope.speed_mps.min, 0.3);
}

#[test]
fn a_reply_that_passes_the_three_checks_is_flown() {
    let mut flight = flight();
    flight.observe(on_the_pad());
    let taken = flight.take_reply("Cleared for takeoff.", &reply(Directive::Takeoff {}), 0.0);
    assert_eq!(taken, Ok(Directive::Takeoff {}));
    let step = flight.step(0.1).expect("a state was observed");
    assert_ne!(step.phase, Phase::Idle);
}

#[test]
fn the_envelope_check_refuses_a_fix_that_is_not_offered() {
    let mut flight = flight();
    let taken = flight.take_reply("Proceed direct ZULU.", &reply(direct_to("ZULU")), 0.0);
    assert_eq!(taken, Err(Refusal::UnknownFix("ZULU".to_owned())));
    assert_eq!(flight.phase(), Phase::Idle);
}

#[test]
fn the_grounding_check_refuses_an_offered_fix_that_the_message_does_not_say() {
    let mut flight = flight();
    flight.observe(on_the_pad());
    let taken = flight.take_reply("Proceed direct ZULU.", &reply(direct_to("ALPHA")), 0.0);
    assert!(
        matches!(taken, Err(Refusal::NotInMessage { slot: "fix", .. })),
        "unexpected result: {taken:?}"
    );
    assert_eq!(
        flight.phase(),
        Phase::Idle,
        "a refused reply changes nothing"
    );
}

#[test]
fn a_step_needs_a_state() {
    let mut flight = flight();
    assert!(flight.step(0.0).is_none());
    flight.observe(on_the_pad());
    assert!(flight.step(0.0).is_some());
}

//! Document-level checks for the heading, speed and go-around actions.

use core::f64::consts::{FRAC_PI_2, TAU};

use crate::{
    CodecError, ExecutionTarget, FlightAction, MissionAction, MissionCapability, MissionDocument,
    TransportLane, TurnDirection, ValidationError,
};

use super::{assert_unknown_field_rejected, document_for};

fn heading(true_heading_rad: f64) -> MissionAction {
    MissionAction::Flight(FlightAction::Heading {
        true_heading_rad,
        turn: TurnDirection::Left,
    })
}

fn speed(speed_mps: f64) -> MissionAction {
    MissionAction::Flight(FlightAction::Speed { speed_mps })
}

fn go_around(target_altitude_m: f64) -> MissionAction {
    MissionAction::Flight(FlightAction::GoAround { target_altitude_m })
}

fn vector_actions() -> [MissionAction; 3] {
    [heading(FRAC_PI_2), speed(4.5), go_around(30.0)]
}

fn assert_refused(action: MissionAction, accept: impl FnOnce(&ValidationError) -> bool) {
    let result = document_for(action, ExecutionTarget::RealVehicle);
    match &result {
        Err(CodecError::Validation(error)) if accept(error) => {}
        _ => panic!("unexpected result: {result:?}"),
    }
}

#[test]
fn each_vector_action_round_trips_through_canonical_json() {
    for action in vector_actions() {
        let document = document_for(action.clone(), ExecutionTarget::RealVehicle)
            .expect("a vector action must be valid on a real vehicle");
        let bytes = document.to_canonical_json().expect("encode");
        let decoded = MissionDocument::from_json(&bytes).expect("decode");
        assert_eq!(decoded.phases[0].action, action);
        assert_eq!(decoded.to_canonical_json().expect("re-encode"), bytes);
    }
}

#[test]
fn vector_actions_have_stable_wire_names() {
    let document = document_for(heading(FRAC_PI_2), ExecutionTarget::RealVehicle).expect("valid");
    let value = serde_json::to_value(&document).expect("encode value");
    let action = &value["phases"][0]["action"];
    assert_eq!(action["domain"], "flight");
    assert_eq!(action["action"]["kind"], "heading");
    assert_eq!(action["action"]["turn"], "left");
    assert_eq!(action["action"]["true_heading_rad"], FRAC_PI_2);
    let names: Vec<_> = vector_actions().iter().map(MissionAction::name).collect();
    assert_eq!(
        names,
        ["flight.heading", "flight.speed", "flight.go_around"]
    );
}

#[test]
fn vector_actions_use_the_operational_lane_and_flight_control() {
    for action in vector_actions() {
        assert_eq!(action.transport_lane(), TransportLane::Operational);
        assert_eq!(
            action.required_capability(),
            Some(MissionCapability::FlightControl)
        );
    }
}

#[test]
fn a_vector_action_without_flight_control_is_refused() {
    let mut document =
        document_for(go_around(30.0), ExecutionTarget::RealVehicle).expect("valid document");
    document.phases[0]
        .required_capabilities
        .retain(|value| *value != MissionCapability::FlightControl);
    assert!(matches!(
        document.validate(),
        Err(ValidationError::UndeclaredCapability {
            capability: MissionCapability::FlightControl,
            ..
        })
    ));
}

#[test]
fn a_heading_has_one_encoding() {
    document_for(heading(0.0), ExecutionTarget::RealVehicle).expect("north is a heading");
    document_for(heading(TAU.next_down()), ExecutionTarget::RealVehicle)
        .expect("the largest value below a full turn is a heading");
    for refused in [TAU, -0.1, 7.0] {
        assert_refused(heading(refused), |error| {
            matches!(error, ValidationError::OutOfRange { field, .. }
                if field.ends_with("action.true_heading_rad"))
        });
    }
    assert_refused(heading(f64::NAN), |error| {
        matches!(error, ValidationError::NonFinite { .. })
    });
}

#[test]
fn a_speed_is_greater_than_zero() {
    for refused in [0.0, -1.0] {
        assert_refused(speed(refused), |error| {
            matches!(error, ValidationError::NotPositive { field, .. }
                if field.ends_with("action.speed_mps"))
        });
    }
    assert_refused(speed(f64::INFINITY), |error| {
        matches!(error, ValidationError::NonFinite { .. })
    });
}

#[test]
fn a_go_around_altitude_is_finite() {
    assert_refused(go_around(f64::NAN), |error| {
        matches!(error, ValidationError::NonFinite { field }
            if field.ends_with("action.target_altitude_m"))
    });
}

#[test]
fn the_decoder_refuses_an_unknown_turn_side_and_an_unknown_field() {
    let document = document_for(heading(FRAC_PI_2), ExecutionTarget::RealVehicle).expect("valid");
    assert_unknown_field_rejected(&document, |value| {
        value["phases"][0]["action"]["action"]["turn"] = serde_json::Value::from("up");
    });
    let document = document_for(go_around(30.0), ExecutionTarget::RealVehicle).expect("valid");
    assert_unknown_field_rejected(&document, |value| {
        value["phases"][0]["action"]["action"]["unknown"] = serde_json::Value::Bool(true);
    });
}

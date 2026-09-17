//! Engine-level checks for the heading and go-around actions.

use core::f64::consts::PI;

use crate::{
    DirectivePurpose, FlightAction, MissionAction, MissionCapability, MissionCondition,
    MissionDirective, MissionObservation, NavigationCondition, TurnDirection, VehicleCondition,
};

use super::support::{document, engine, phase, succeeded, tick};

const GO_AROUND_ALTITUDE_M: f64 = 30.0;

fn flight_phase(id: &str, action: FlightAction) -> crate::MissionPhase {
    let mut phase = phase(id);
    phase.action = MissionAction::Flight(action);
    phase
        .required_capabilities
        .push(MissionCapability::FlightControl);
    phase
}

fn flight_action(directive: &MissionDirective) -> &FlightAction {
    let MissionDirective::Flight(directive) = directive else {
        panic!("expected a flight directive: {directive:?}");
    };
    &directive.action
}

#[test]
fn the_engine_gives_a_heading_to_the_host_without_a_change() {
    let action = FlightAction::Heading {
        true_heading_rad: PI,
        turn: TurnDirection::Right,
    };
    let mut engine = engine(document(vec![flight_phase("vector", action.clone())]));
    let output = tick(&mut engine, 0, 0, MissionObservation::default(), Vec::new());
    assert_eq!(output.directives.len(), 1);
    assert_eq!(flight_action(&output.directives[0]), &action);
}

#[test]
fn an_aborted_landing_gives_the_go_around_as_its_cleanup() {
    let mut landing = flight_phase("landing", FlightAction::Land {});
    landing
        .required_capabilities
        .push(MissionCapability::NavigationState);
    landing.completion_conditions =
        vec![MissionCondition::Vehicle(VehicleCondition::GroundContact {
            expected: true,
        })];
    landing.abort_conditions = vec![MissionCondition::Navigation(
        NavigationCondition::GuidanceValid { expected: false },
    )];
    landing.cleanup_actions = vec![MissionAction::Flight(FlightAction::GoAround {
        target_altitude_m: GO_AROUND_ALTITUDE_M,
    })];
    let mut engine = engine(document(vec![landing]));

    let land = tick(&mut engine, 0, 0, guidance(true), Vec::new());
    assert_eq!(flight_action(&land.directives[0]), &FlightAction::Land {});
    let descending = tick(&mut engine, 1, 1, guidance(true), vec![succeeded(&land)]);
    assert!(descending.directives.is_empty());

    let aborted = tick(&mut engine, 2, 2, guidance(false), Vec::new());
    assert_eq!(aborted.directives.len(), 1);
    assert_eq!(
        aborted.directives[0].context().purpose,
        DirectivePurpose::Cleanup { cleanup_index: 0 }
    );
    assert_eq!(
        flight_action(&aborted.directives[0]),
        &FlightAction::GoAround {
            target_altitude_m: GO_AROUND_ALTITUDE_M
        }
    );
}

fn guidance(valid: bool) -> MissionObservation {
    let mut observation = MissionObservation::default();
    observation.navigation.guidance_valid = Some(valid);
    observation
}

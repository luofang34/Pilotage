#![allow(clippy::expect_used, clippy::panic)]

use core::f64::consts::PI;

use pilotage_mission_core::{
    Digest, ExecutionPolicy, ExecutionTarget, FlightAction, FlightPlanReference, MissionAction,
    MissionCapability, MissionDocument, MissionPhase, NavigationDataIdentity,
    TurnDirection as MissionTurn,
};

use super::{LoweringContext, LoweringError, flight_actions};
use crate::directive::{Arrival, Directive, HoldPoint, TurnDirection};

fn navigation_data() -> NavigationDataIdentity {
    NavigationDataIdentity {
        cycle: "2608".to_owned(),
        snapshot_id: "nav-1".to_owned(),
        snapshot_digest: Digest::from_bytes([1; 32]),
    }
}

/// A plan owner that knows `HOME`, `BRAVO` and `RNAV27`.
fn plan_for(name: &str) -> Option<FlightPlanReference> {
    ["HOME", "BRAVO", "RNAV27"]
        .contains(&name)
        .then(|| FlightPlanReference {
            plan_id: format!("plan-{name}"),
            plan_content_digest: Digest::from_bytes([2; 32]),
            navigation_data_identity: navigation_data(),
        })
}

fn context() -> LoweringContext<'static> {
    LoweringContext {
        launch_altitude_m: 100.0,
        cruise_height_m: 5.0,
        plan_for: &plan_for,
    }
}

fn names(directive: &Directive) -> Vec<String> {
    flight_actions(directive, &context())
        .expect("the directive lowers")
        .iter()
        .map(|action| {
            serde_json::to_value(action).expect("encode")["kind"]
                .as_str()
                .expect("a kind")
                .to_owned()
        })
        .collect()
}

#[test]
fn each_directive_has_the_sequence_that_the_decision_record_states() {
    let direct = Directive::DirectTo {
        fix: "BRAVO".to_owned(),
        on_arrival: Arrival::Land,
    };
    let hold = Directive::Hold {
        point: HoldPoint::Fix("BRAVO".to_owned()),
    };
    let join = Directive::JoinProcedure {
        procedure: "RNAV27".to_owned(),
    };
    assert_eq!(names(&Directive::Takeoff {}), ["arm", "climb"]);
    assert_eq!(names(&direct), ["follow_plan", "land"]);
    assert_eq!(names(&hold), ["follow_plan", "maintain_target"]);
    assert_eq!(
        names(&Directive::Hold {
            point: HoldPoint::PresentPosition
        }),
        ["maintain_target"]
    );
    assert_eq!(names(&join), ["follow_plan"]);
    assert_eq!(names(&Directive::Land {}), ["land"]);
    assert_eq!(names(&Directive::GoAround {}), ["go_around"]);
    assert_eq!(names(&Directive::ReturnToBase {}), ["follow_plan", "land"]);
    let unable = Directive::Unable {
        reason: "not an instruction".to_owned(),
    };
    assert_eq!(names(&unable), Vec::<String>::new());
}

#[test]
fn a_heading_in_degrees_becomes_a_true_heading_in_radians_with_its_turn_side() {
    let directive = Directive::Heading {
        degrees: 270,
        turn: TurnDirection::Left,
    };
    let actions = flight_actions(&directive, &context()).expect("lowers");
    let [
        FlightAction::Heading {
            true_heading_rad,
            turn,
        },
    ] = actions.as_slice()
    else {
        panic!("unexpected actions: {actions:?}");
    };
    assert!((true_heading_rad - 1.5 * PI).abs() < 1e-12);
    assert_eq!(*turn, MissionTurn::Left);
}

#[test]
fn heights_are_above_the_launch_point_and_altitudes_are_absolute() {
    let climb =
        flight_actions(&Directive::Altitude { height_m: 12.0 }, &context()).expect("lowers");
    assert_eq!(
        climb,
        [FlightAction::Climb {
            target_altitude_m: 112.0
        }]
    );
    let go_around = flight_actions(&Directive::GoAround {}, &context()).expect("lowers");
    assert_eq!(
        go_around,
        [FlightAction::GoAround {
            target_altitude_m: 105.0
        }]
    );
    let speed = flight_actions(&Directive::Speed { speed_mps: 2.0 }, &context()).expect("lowers");
    assert_eq!(speed, [FlightAction::Speed { speed_mps: 2.0 }]);
}

#[test]
fn a_fix_with_no_plan_has_no_lowering() {
    let directive = Directive::DirectTo {
        fix: "ZULU".to_owned(),
        on_arrival: Arrival::Hold,
    };
    assert_eq!(
        flight_actions(&directive, &context()),
        Err(LoweringError::NoPlan {
            name: "ZULU".to_owned()
        })
    );
}

/// The mission core accepts each lowered action in a document. A heading of
/// 359 degrees is the largest that a directive can carry.
#[test]
fn the_mission_core_admits_each_lowered_action() {
    let directives = [
        Directive::Takeoff {},
        Directive::Heading {
            degrees: 359,
            turn: TurnDirection::Shortest,
        },
        Directive::Speed { speed_mps: 0.3 },
        Directive::GoAround {},
        Directive::ReturnToBase {},
    ];
    let mut phases = Vec::new();
    for directive in &directives {
        for action in flight_actions(directive, &context()).expect("lowers") {
            phases.push(MissionPhase {
                id: format!("phase-{}", phases.len()),
                required_capabilities: vec![
                    MissionCapability::SimulatorTime,
                    MissionCapability::ArmDisarm,
                    MissionCapability::FlightControl,
                    MissionCapability::FlightPlan,
                ],
                entry_conditions: Vec::new(),
                action: MissionAction::Flight(action),
                cleanup_actions: Vec::new(),
                completion_conditions: Vec::new(),
                abort_conditions: Vec::new(),
                simulator_time_deadline_ns: 1_000_000,
            });
        }
    }
    assert_eq!(phases.len(), 7);
    MissionDocument::new(
        "lowered-directives".to_owned(),
        navigation_data(),
        ExecutionPolicy {
            target: ExecutionTarget::RealVehicle,
            retry_limit: 1,
            receipt_timeout_ns: 500_000,
        },
        phases,
    )
    .expect("the mission core admits each lowered action");
}

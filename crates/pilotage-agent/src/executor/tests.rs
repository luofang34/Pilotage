//! Executor behaviour, one decision for each test.

#![allow(clippy::panic)]

mod no_effect;

use super::{Discrete, Executor, FlightLimits, Phase};
use crate::directive::{Arrival, Directive, HoldPoint, TurnDirection};
use crate::scenario::Scenario;
use crate::state::VehicleState;

const LIMITS: FlightLimits = FlightLimits {
    cruise_height_m: 5.0,
    cruise_speed_mps: 2.4,
    arrival_radius_m: 1.5,
    max_range_m: 40.0,
    max_linear_mps: 3.0,
    disarm_offered: true,
};

const SCENARIO: &str = r#"{
  "id": "t",
  "fixes": {"ALPHA": {"north_m": 15, "east_m": 0}, "BRAVO": {"north_m": 0, "east_m": 15},
            "FAF": {"north_m": 0, "east_m": 30}},
  "procedures": {"RNAV27": {"fixes": ["FAF", "BRAVO", "HOME"], "then": "land"}},
  "cruise_height_m": 5, "arrival_radius_m": 1.5, "max_range_m": 40,
  "height_m": {"min": 2, "max": 30},
  "expect": {"end": {"state": "checkpoints_only"}, "timeout_s": 60}
}"#;

fn executor() -> Executor {
    match Scenario::parse(SCENARIO) {
        Ok(scenario) => Executor::new(LIMITS, &scenario),
        Err(error) => panic!("the test scenario parses: {error}"),
    }
}

fn state(north_m: f64, east_m: f64, height_m: f64, armed: bool) -> VehicleState {
    VehicleState {
        north_m,
        east_m,
        vel_north_mps: 0.0,
        vel_east_mps: 0.0,
        height_m: Some(height_m),
        climb_mps: 0.0,
        yaw_rad: 0.0,
        armed: Some(armed),
    }
}

fn direct(fix: &str, on_arrival: Arrival) -> Directive {
    Directive::DirectTo {
        fix: fix.to_owned(),
        on_arrival,
    }
}

/// An executor in the air at the origin, flying `directive`.
fn airborne(directive: &Directive) -> Executor {
    let mut executor = executor();
    let ground = state(0.0, 0.0, 0.0, false);
    assert!(executor.accept(directive, Some(&ground), 0.0).is_ok());
    executor.step(&ground, 0.0);
    executor.step(&state(0.0, 0.0, 0.0, true), 0.1);
    executor.step(&state(0.0, 0.0, 5.0, true), 1.0);
    executor
}

#[test]
fn nothing_happens_without_a_directive() {
    let mut executor = executor();
    let step = executor.step(&state(0.0, 0.0, 0.0, false), 0.0);
    assert_eq!((step.phase, step.action), (Phase::Idle, None));
}

#[test]
fn a_directive_on_the_ground_arms_then_climbs_then_flies() {
    let mut executor = executor();
    let ground = state(0.0, 0.0, 0.0, false);
    assert!(
        executor
            .accept(&direct("ALPHA", Arrival::Land), Some(&ground), 0.0)
            .is_ok()
    );
    assert_eq!(executor.step(&ground, 0.0).phase, Phase::Arming);
    assert_eq!(executor.step(&ground, 0.05).action, Some(Discrete::Arm));
    assert_eq!(
        executor.step(&ground, 1.0).action,
        None,
        "no repeat before the interval"
    );
    assert_eq!(executor.step(&ground, 2.1).action, Some(Discrete::Arm));
    assert_eq!(
        executor.step(&state(0.0, 0.0, 0.0, true), 2.2).phase,
        Phase::Climb
    );
    assert!(
        executor
            .step(&state(0.0, 0.0, 1.0, true), 2.3)
            .demand
            .throttle
            > 0.5
    );
    assert_eq!(
        executor.step(&state(0.0, 0.0, 4.8, true), 7.0).phase,
        Phase::Enroute
    );
    assert!(executor.step(&state(0.0, 0.0, 5.0, true), 7.1).demand.pitch > 0.9);
}

#[test]
fn a_takeoff_climbs_and_holds_where_the_climb_ends() {
    let mut executor = airborne(&Directive::Takeoff {});
    let step = executor.step(&state(0.2, -0.1, 5.0, true), 1.1);
    assert_eq!(step.phase, Phase::Holding);
    assert!(step.demand.pitch.abs() < 0.2 && step.demand.yaw.abs() < f32::EPSILON);
}

#[test]
fn arrival_with_land_descends_then_disarms_on_the_ground() {
    let mut executor = airborne(&direct("ALPHA", Arrival::Land));
    let mut fast = state(14.5, 0.0, 5.0, true);
    fast.vel_north_mps = 2.0;
    let passing = executor.step(&fast, 8.0);
    assert_eq!(
        passing.phase,
        Phase::Enroute,
        "inside the radius, and too fast"
    );
    assert!(
        passing.demand.pitch < 0.0,
        "brakes against the closing speed"
    );

    assert_eq!(
        executor.step(&state(14.8, 0.0, 5.0, true), 9.0).phase,
        Phase::Descending
    );
    let down = executor.step(&state(14.8, 0.0, 3.0, true), 10.0);
    assert!(down.demand.throttle < 0.0);
    assert_eq!(down.action, None, "no disarm in the air");
    assert_eq!(
        executor.step(&state(14.8, 0.0, 0.2, true), 14.0).phase,
        Phase::Disarming
    );
    assert_eq!(
        executor.step(&state(14.8, 0.0, 0.2, true), 14.1).action,
        Some(Discrete::Disarm)
    );
    assert_eq!(
        executor.step(&state(14.8, 0.0, 0.2, false), 14.5).phase,
        Phase::Landed
    );
    assert_eq!(
        executor.step(&state(14.8, 0.0, 0.2, false), 15.0).action,
        None
    );
}

#[test]
fn a_vehicle_that_offers_no_disarm_is_landed_at_touchdown() {
    let scenario = Scenario::parse(SCENARIO).ok();
    let Some(scenario) = scenario else {
        panic!("the test scenario parses");
    };
    let mut executor = Executor::new(
        FlightLimits {
            disarm_offered: false,
            ..LIMITS
        },
        &scenario,
    );
    let ground = state(0.0, 0.0, 0.0, false);
    assert!(
        executor
            .accept(&direct("ALPHA", Arrival::Land), Some(&ground), 0.0)
            .is_ok()
    );
    executor.step(&ground, 0.0);
    executor.step(&state(0.0, 0.0, 0.0, true), 0.1);
    executor.step(&state(0.0, 0.0, 5.0, true), 1.0);
    executor.step(&state(14.8, 0.0, 5.0, true), 9.0);
    let down = executor.step(&state(14.8, 0.0, 0.2, true), 14.0);
    assert_eq!((down.phase, down.action), (Phase::Landed, None));
}

#[test]
fn a_new_directive_in_flight_turns_the_vehicle_and_restarts_the_flying_time() {
    let mut executor = airborne(&direct("ALPHA", Arrival::Hold));
    executor.step(&state(5.0, 0.0, 5.0, true), 4.0);
    assert!(executor.flying_seconds(4.0).is_some_and(|s| s > 2.9));
    let here = state(5.0, 0.0, 5.0, true);
    assert!(
        executor
            .accept(&direct("BRAVO", Arrival::Land), Some(&here), 4.0)
            .is_ok()
    );
    let step = executor.step(&here, 4.05);
    assert_eq!(step.phase, Phase::Enroute);
    assert!(step.demand.yaw > 0.9, "BRAVO is to the right");
    assert!(executor.flying_seconds(5.0).is_some_and(|s| s < 1.1));
}

#[test]
fn a_heading_flies_until_the_range_limit_and_then_holds() {
    let heading = Directive::Heading {
        degrees: 0,
        turn: TurnDirection::Shortest,
    };
    let mut executor = airborne(&heading);
    let cruising = executor.step(&state(10.0, 0.0, 5.0, true), 5.0);
    assert_eq!(cruising.phase, Phase::OnHeading);
    assert!(cruising.demand.pitch > 0.9);
    let beyond = executor.step(&state(40.5, 0.0, 5.0, true), 20.0);
    assert_eq!(
        beyond.phase,
        Phase::RangeHold,
        "no directive followed, so it stops"
    );
    let mut moving = state(41.0, 0.0, 5.0, true);
    moving.vel_north_mps = 2.0;
    assert!(
        executor.step(&moving, 20.1).demand.pitch < 0.0,
        "brakes at the limit"
    );
}

#[test]
fn an_altitude_directive_changes_the_held_height() {
    let mut executor = airborne(&Directive::Takeoff {});
    let here = state(0.0, 0.0, 5.0, true);
    assert!(
        executor
            .accept(&Directive::Altitude { height_m: 12.0 }, Some(&here), 2.0)
            .is_ok()
    );
    assert!(
        executor.step(&here, 2.1).demand.throttle > 0.4,
        "climbs to the new height"
    );
    assert!(
        executor
            .step(&state(0.0, 0.0, 12.0, true), 20.0)
            .demand
            .throttle
            .abs()
            < 0.05
    );
}

#[test]
fn a_procedure_passes_its_fixes_in_sequence_and_lands_at_the_last() {
    let join = Directive::JoinProcedure {
        procedure: "RNAV27".to_owned(),
    };
    let mut executor = airborne(&join);
    // FAF is 30 m east. A fast pass inside its radius counts, because it is
    // not the last fix.
    let mut at_faf = state(0.0, 29.5, 5.0, true);
    at_faf.vel_east_mps = 2.0;
    at_faf.yaw_rad = std::f64::consts::FRAC_PI_2;
    executor.step(&at_faf, 15.0);
    let to_bravo = executor.step(&at_faf, 15.05);
    assert_eq!(to_bravo.phase, Phase::Enroute);
    assert!(to_bravo.demand.yaw.abs() > 0.9, "turns back west for BRAVO");
    executor.step(&state(0.0, 15.2, 5.0, true), 25.0);
    assert_eq!(
        executor.step(&state(0.3, 0.2, 5.0, true), 35.0).phase,
        Phase::Descending
    );
}

#[test]
fn a_go_around_stops_a_descent_and_a_land_starts_one() {
    let mut executor = airborne(&Directive::Takeoff {});
    let here = state(3.0, 4.0, 5.0, true);
    executor.step(&here, 1.1);
    assert!(
        executor
            .accept(&Directive::Land {}, Some(&here), 2.0)
            .is_ok()
    );
    assert_eq!(executor.step(&here, 2.1).phase, Phase::Descending);
    let low = state(3.0, 4.0, 2.0, true);
    assert!(
        executor
            .accept(&Directive::GoAround {}, Some(&low), 5.0)
            .is_ok()
    );
    let climbing = executor.step(&low, 5.1);
    assert_eq!(climbing.phase, Phase::Holding);
    assert!(
        climbing.demand.throttle > 0.4,
        "climbs back to the commanded height"
    );
}

#[test]
fn a_hold_at_present_position_stays_where_the_directive_arrived() {
    let mut executor = airborne(&direct("ALPHA", Arrival::Land));
    let here = state(6.0, 0.5, 5.0, true);
    let hold = Directive::Hold {
        point: HoldPoint::PresentPosition,
    };
    assert!(executor.accept(&hold, Some(&here), 4.0).is_ok());
    assert_eq!(executor.step(&here, 4.1).phase, Phase::Holding);
    let drifted = executor.step(&state(7.0, 0.5, 5.0, true), 5.0);
    assert!(drifted.demand.pitch < 0.0, "comes back to the hold point");
}

#[test]
fn an_unknown_fix_is_refused_and_changes_nothing() {
    let mut executor = airborne(&direct("ALPHA", Arrival::Hold));
    let here = state(5.0, 0.0, 5.0, true);
    assert!(
        executor
            .accept(&direct("ZULU", Arrival::Land), Some(&here), 4.0)
            .is_err()
    );
    assert_eq!(executor.step(&here, 4.1).phase, Phase::Enroute);
}

#[test]
fn unable_flies_nothing() {
    let mut executor = executor();
    let ground = state(0.0, 0.0, 0.0, false);
    let unable = Directive::Unable {
        reason: "not an instruction".to_owned(),
    };
    assert!(executor.accept(&unable, Some(&ground), 0.0).is_ok());
    assert_eq!(executor.step(&ground, 0.1).phase, Phase::Idle);
}

#[test]
fn a_directive_that_arrives_during_disarm_is_flown_after_it() {
    let mut executor = airborne(&direct("ALPHA", Arrival::Land));
    executor.step(&state(14.8, 0.0, 5.0, true), 9.0);
    executor.step(&state(14.8, 0.0, 0.2, true), 14.0);
    assert_eq!(executor.phase(), Phase::Disarming);
    let down = state(14.8, 0.0, 0.2, false);
    assert!(
        executor
            .accept(&Directive::ReturnToBase {}, Some(&down), 14.2)
            .is_ok()
    );
    assert_eq!(executor.step(&down, 14.5).phase, Phase::Landed);
    assert_eq!(executor.step(&down, 14.6).phase, Phase::Arming);
}

#[test]
fn a_climb_that_stays_on_the_ground_forces_a_new_arm_request() {
    // The report says armed, and it is old: the flight controller restarted.
    let mut executor = executor();
    let stale = state(0.0, 0.0, 0.0, true);
    assert!(
        executor
            .accept(&direct("ALPHA", Arrival::Land), Some(&stale), 0.0)
            .is_ok()
    );
    executor.step(&stale, 0.0);
    assert_eq!(
        executor.step(&stale, 0.05).phase,
        Phase::Climb,
        "it believes the report"
    );
    // The stall time counts from the first frame of the climb.
    assert_eq!(executor.step(&stale, 0.1).phase, Phase::Climb);
    assert_eq!(executor.step(&stale, 2.9).phase, Phase::Climb);
    assert_eq!(
        executor.step(&stale, 3.1).phase,
        Phase::Arming,
        "the climb did not leave the ground"
    );
    assert_eq!(
        executor.step(&stale, 3.15).action,
        Some(Discrete::Arm),
        "armed report or not"
    );
    executor.on_action_result(Discrete::Arm, true);
    assert_eq!(executor.step(&stale, 3.3).phase, Phase::Climb);
    assert_eq!(
        executor.step(&state(0.0, 0.0, 1.0, true), 4.0).phase,
        Phase::Climb
    );
    assert_eq!(
        executor.step(&state(0.0, 0.0, 1.2, true), 9.0).phase,
        Phase::Climb,
        "in the air is not stalled"
    );
}

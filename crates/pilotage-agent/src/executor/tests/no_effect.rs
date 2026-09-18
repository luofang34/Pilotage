//! A directive that changes nothing is refused, and the phases that a hold,
//! a landing or a return can end.

use super::{LIMITS, Phase, airborne, direct, executor, state};
use crate::directive::{Arrival, Directive, HoldPoint, TurnDirection};
use crate::model_port::Refusal;

const HOLD_HERE: Directive = Directive::Hold {
    point: HoldPoint::PresentPosition,
};

fn no_effect(result: Result<(), Refusal>) -> bool {
    matches!(result, Err(Refusal::NoEffect { .. }))
}

#[test]
fn a_return_to_base_on_the_ground_is_refused_and_flies_nothing() {
    let mut executor = executor();
    let ground = state(0.0, 0.0, 0.0, false);
    let result = executor.accept(&Directive::ReturnToBase {}, Some(&ground), 0.0);
    assert!(
        no_effect(result),
        "a return from the ground is a takeoff that nobody asked for"
    );
    let step = executor.step(&ground, 0.1);
    assert_eq!((step.phase, step.action), (Phase::Idle, None));
}

#[test]
fn a_takeoff_in_the_air_and_a_landing_on_the_ground_are_refused() {
    let mut flying = airborne(&Directive::Takeoff {});
    let here = state(0.0, 0.0, 5.0, true);
    assert!(no_effect(flying.accept(
        &Directive::Takeoff {},
        Some(&here),
        2.0
    )));
    assert_eq!(flying.step(&here, 2.1).phase, Phase::Holding);
    let mut grounded = executor();
    let ground = state(0.0, 0.0, 0.0, false);
    for directive in [Directive::Land {}, Directive::GoAround {}, HOLD_HERE] {
        assert!(
            no_effect(grounded.accept(&directive, Some(&ground), 0.0)),
            "{directive:?}"
        );
    }
    assert_eq!(grounded.step(&ground, 0.1).phase, Phase::Idle);
}

#[test]
fn a_landing_during_the_climb_descends_at_once() {
    let mut executor = executor();
    let ground = state(0.0, 0.0, 0.0, false);
    assert!(
        executor
            .accept(&direct("ALPHA", Arrival::Hold), Some(&ground), 0.0)
            .is_ok()
    );
    executor.step(&ground, 0.0);
    let climbing = state(0.0, 0.0, 1.0, true);
    assert_eq!(executor.step(&climbing, 0.1).phase, Phase::Climb);
    assert!(
        executor
            .accept(&Directive::Land {}, Some(&climbing), 0.2)
            .is_ok()
    );
    let step = executor.step(&climbing, 0.3);
    assert_eq!(step.phase, Phase::Descending);
    assert!(
        step.demand.throttle < 0.0,
        "descends and does not finish the climb"
    );
}

#[test]
fn a_hold_during_the_climb_holds_at_the_present_height() {
    let mut executor = executor();
    let ground = state(0.0, 0.0, 0.0, false);
    assert!(
        executor
            .accept(&direct("ALPHA", Arrival::Hold), Some(&ground), 0.0)
            .is_ok()
    );
    executor.step(&ground, 0.0);
    let climbing = state(0.0, 0.0, 2.0, true);
    assert_eq!(executor.step(&climbing, 0.1).phase, Phase::Climb);
    assert!(executor.accept(&HOLD_HERE, Some(&climbing), 0.2).is_ok());
    assert_eq!(executor.step(&climbing, 0.3).phase, Phase::Holding);
    let higher = executor.step(&state(0.0, 0.0, 3.0, true), 0.4);
    assert!(higher.demand.throttle < 0.0, "the climb ended at 2 m");
    assert!(higher.demand.pitch.abs() < 0.05, "ALPHA is forgotten");
}

#[test]
fn a_hold_or_landing_during_arming_cancels_the_takeoff() {
    for directive in [HOLD_HERE, Directive::Land {}, Directive::ReturnToBase {}] {
        let mut executor = executor();
        let ground = state(0.0, 0.0, 0.0, false);
        assert!(
            executor
                .accept(&Directive::Takeoff {}, Some(&ground), 0.0)
                .is_ok()
        );
        assert_eq!(executor.step(&ground, 0.0).phase, Phase::Arming);
        assert!(
            executor.accept(&directive, Some(&ground), 0.1).is_ok(),
            "{directive:?}"
        );
        let step = executor.step(&state(0.0, 0.0, 0.0, true), 0.2);
        assert_eq!(step.phase, Phase::Disarming, "{directive:?}");
        assert!(step.demand.throttle.abs() < 0.01, "no climb demand");
        let down = executor.step(&ground, 0.3);
        assert_eq!(down.phase, Phase::Landed);
    }
}

#[test]
fn a_new_height_during_a_descent_ends_the_descent() {
    let mut executor = airborne(&Directive::Takeoff {});
    let here = state(3.0, 4.0, 5.0, true);
    executor.step(&here, 1.1);
    assert!(
        executor
            .accept(&Directive::Land {}, Some(&here), 2.0)
            .is_ok()
    );
    let low = state(3.0, 4.0, 2.0, true);
    assert_eq!(executor.step(&low, 2.1).phase, Phase::Descending);
    assert!(
        executor
            .accept(&Directive::Altitude { height_m: 8.0 }, Some(&low), 3.0)
            .is_ok()
    );
    let step = executor.step(&low, 3.1);
    assert_ne!(step.phase, Phase::Descending);
    assert!(step.demand.throttle > 0.0, "climbs toward 8 m");
}

#[test]
fn a_go_around_in_flight_climbs_to_the_cruise_height_and_holds() {
    let mut executor = airborne(&direct("ALPHA", Arrival::Land));
    let low = state(6.0, 0.0, 3.0, true);
    executor.step(&low, 1.1);
    assert!(
        executor
            .accept(&Directive::Altitude { height_m: 3.0 }, Some(&low), 1.2)
            .is_ok()
    );
    assert!(
        executor
            .accept(&Directive::GoAround {}, Some(&low), 2.0)
            .is_ok()
    );
    let step = executor.step(&low, 2.1);
    assert_eq!(step.phase, Phase::Holding);
    assert!(
        step.demand.throttle > 0.0,
        "back to {} m",
        LIMITS.cruise_height_m
    );
}

#[test]
fn a_heading_toward_home_leaves_the_range_hold() {
    let north = Directive::Heading {
        degrees: 0,
        turn: TurnDirection::Shortest,
    };
    let mut executor = airborne(&north);
    let beyond = state(40.5, 0.0, 5.0, true);
    executor.step(&state(10.0, 0.0, 5.0, true), 5.0);
    assert_eq!(executor.step(&beyond, 20.0).phase, Phase::RangeHold);
    let south = Directive::Heading {
        degrees: 180,
        turn: TurnDirection::Left,
    };
    assert!(executor.accept(&south, Some(&beyond), 21.0).is_ok());
    let step = executor.step(&beyond, 21.1);
    assert_eq!(step.phase, Phase::OnHeading, "the way home is not held");
    assert!(
        step.demand.pitch > 0.0 || step.demand.yaw.abs() > 0.0,
        "turns or moves"
    );
    assert!(executor.accept(&north, Some(&beyond), 22.0).is_ok());
    assert_eq!(
        executor.step(&beyond, 22.1).phase,
        Phase::RangeHold,
        "away from home holds"
    );
}

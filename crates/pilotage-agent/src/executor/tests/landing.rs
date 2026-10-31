//! Touchdown is the flight controller's judgement when it reports one, and
//! the end of the descent when it does not. A height alone is neither.

use super::{airborne, direct, state};
use crate::directive::Arrival;
use crate::executor::Phase;
use crate::state::VehicleState;

fn descending_at(height_m: f64, on_ground: Option<bool>) -> VehicleState {
    VehicleState {
        climb_mps: -0.5,
        on_ground,
        ..state(14.8, 0.0, height_m, true)
    }
}

fn over_the_fix() -> super::Executor {
    let mut executor = airborne(&direct("ALPHA", Arrival::Land));
    executor.step(&state(14.8, 0.0, 5.0, true), 9.0);
    assert_eq!(executor.phase(), Phase::Descending);
    executor
}

#[test]
fn a_low_estimate_is_not_a_touchdown_while_the_detector_reads_in_the_air() {
    let mut executor = over_the_fix();
    let step = executor.step(&descending_at(0.1, Some(false)), 14.0);
    assert_eq!(step.phase, Phase::Descending);
    assert!(step.demand.throttle < 0.0, "the descent continues");
    assert_eq!(step.action, None);
}

#[test]
fn a_vehicle_stopped_by_the_ground_descends_until_the_detector_agrees() {
    let mut executor = over_the_fix();
    let stopped = VehicleState {
        on_ground: Some(false),
        ..state(14.8, 0.0, 0.1, true)
    };
    for tick in 0..50 {
        let step = executor.step(&stopped, 14.0 + f64::from(tick) * 0.1);
        assert_eq!(step.phase, Phase::Descending);
    }
    let on_ground = executor.step(&descending_at(0.0, Some(true)), 20.0);
    assert_eq!(on_ground.phase, Phase::Disarming);
}

#[test]
fn without_a_report_a_low_descending_vehicle_keeps_descending() {
    let mut executor = over_the_fix();
    let step = executor.step(&descending_at(0.1, None), 14.0);
    assert_eq!(step.phase, Phase::Descending);
}

#[test]
fn without_a_report_the_end_of_the_descent_is_the_touchdown() {
    let mut executor = over_the_fix();
    let stopped = state(14.8, 0.0, 0.1, true);
    assert_eq!(executor.step(&stopped, 14.0).phase, Phase::Descending);
    assert_eq!(executor.step(&stopped, 15.9).phase, Phase::Descending);
    assert_eq!(executor.step(&stopped, 16.1).phase, Phase::Disarming);
}

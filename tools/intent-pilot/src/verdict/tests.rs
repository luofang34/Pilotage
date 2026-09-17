//! Verifier behaviour. The negative cases are pilots that must not pass.

#![allow(clippy::panic)]

use super::{Verdict, Verifier};
use crate::scenario::{
    Approach, Arrival, Expectation, Intent, OperatorMessage, Scenario, Trigger, Waypoint,
};
use crate::vehicle_state::TruthState;

fn scenario(expect: Expectation) -> Scenario {
    let waypoints = [("ALPHA", 15.0, 0.0), ("BRAVO", 0.0, 15.0)]
        .into_iter()
        .map(|(name, north_m, east_m)| (name.to_owned(), Waypoint { north_m, east_m }))
        .collect();
    Scenario {
        id: "test".to_owned(),
        waypoints,
        cruise_height_m: 5.0,
        arrival_radius_m: 1.5,
        messages: vec![OperatorMessage {
            text: "Go to ALPHA and land.".to_owned(),
            trigger: Trigger::Start,
            means: Intent {
                target: "ALPHA".to_owned(),
                on_arrival: Arrival::Land,
            },
        }],
        expect,
    }
}

fn land_at(target: &str) -> Expectation {
    Expectation {
        final_target: target.to_owned(),
        final_on_arrival: Arrival::Land,
        must_approach: Vec::new(),
        must_not_reach: Vec::new(),
        timeout_s: 60.0,
    }
}

fn truth(north_m: f64, east_m: f64, height_m: f64) -> TruthState {
    TruthState {
        north_m,
        east_m,
        height_m: Some(height_m),
    }
}

/// Feeds `sample` every 0.5 s over `[from_s, to_s]` and returns the first
/// decision.
fn dwell(
    verifier: &mut Verifier,
    sample: TruthState,
    armed: Option<bool>,
    from_s: f64,
    to_s: f64,
) -> Option<Verdict> {
    let mut at = from_s;
    while at <= to_s {
        if let Some(report) = verifier.observe(&sample, armed, at) {
            return Some(report.verdict);
        }
        at += 0.5;
    }
    None
}

fn verifier(expect: Expectation) -> Verifier {
    match Verifier::new(&scenario(expect)) {
        Some(verifier) => verifier,
        None => panic!("the test scenario names only waypoints it defines"),
    }
}

#[test]
fn a_flight_that_lands_disarmed_at_the_expected_waypoint_passes() {
    let mut verifier = verifier(land_at("ALPHA"));
    assert_eq!(
        dwell(&mut verifier, truth(7.0, 0.0, 5.0), Some(true), 0.0, 5.0),
        None
    );
    let verdict = dwell(
        &mut verifier,
        truth(14.6, 0.2, 0.1),
        Some(false),
        20.0,
        24.0,
    );
    assert_eq!(verdict, Some(Verdict::Pass));
}

#[test]
fn a_pilot_that_never_flies_fails_a_flight_that_ends_at_home() {
    let mut verifier = verifier(land_at("HOME"));
    let verdict = dwell(&mut verifier, truth(0.0, 0.0, 0.0), Some(false), 0.0, 30.0);
    assert_eq!(verdict, None, "sitting at HOME is not a completed flight");
    let report = verifier.on_clock(60.0);
    assert_eq!(report.map(|report| report.verdict), Some(Verdict::Fail));
}

#[test]
fn a_pilot_that_hops_at_home_fails_when_a_checkpoint_is_required() {
    let mut expect = land_at("HOME");
    expect.must_approach = vec![Approach {
        target: "BRAVO".to_owned(),
        within_m: 12.0,
    }];
    let mut verifier = verifier(expect);
    dwell(&mut verifier, truth(0.0, 0.0, 5.0), Some(true), 0.0, 5.0);
    let verdict = dwell(&mut verifier, truth(0.0, 0.0, 0.0), Some(false), 10.0, 20.0);
    assert_eq!(verdict, None, "BRAVO was never approached");
    let reasons = verifier.on_clock(60.0).map(|report| report.reasons);
    assert!(reasons.is_some_and(|reasons| reasons.iter().any(|r| r.contains("BRAVO"))));
}

#[test]
fn the_same_flight_passes_once_the_checkpoint_is_flown() {
    let mut expect = land_at("HOME");
    expect.must_approach = vec![Approach {
        target: "BRAVO".to_owned(),
        within_m: 12.0,
    }];
    let mut verifier = verifier(expect);
    dwell(&mut verifier, truth(0.0, 0.0, 5.0), Some(true), 0.0, 2.0);
    dwell(&mut verifier, truth(0.0, 6.0, 5.0), Some(true), 5.0, 6.0);
    let verdict = dwell(&mut verifier, truth(0.1, 0.1, 0.0), Some(false), 20.0, 24.0);
    assert_eq!(verdict, Some(Verdict::Pass));
}

#[test]
fn a_clean_landing_at_the_wrong_waypoint_fails() {
    // The classifier read BRAVO where the author wrote ALPHA.
    let mut verifier = verifier(land_at("ALPHA"));
    dwell(&mut verifier, truth(0.0, 7.0, 5.0), Some(true), 0.0, 5.0);
    let verdict = dwell(
        &mut verifier,
        truth(0.0, 15.0, 0.0),
        Some(false),
        20.0,
        40.0,
    );
    assert_eq!(verdict, None);
    assert_eq!(
        verifier.on_clock(60.0).map(|report| report.verdict),
        Some(Verdict::Fail)
    );
}

#[test]
fn entering_a_forbidden_waypoint_fails_at_once() {
    let mut expect = land_at("BRAVO");
    expect.must_not_reach = vec!["ALPHA".to_owned()];
    let mut verifier = verifier(expect);
    let verdict = dwell(&mut verifier, truth(14.0, 0.0, 5.0), Some(true), 8.0, 8.0);
    assert_eq!(verdict, Some(Verdict::Fail));
}

#[test]
fn a_vehicle_on_the_ground_that_is_still_armed_has_not_landed() {
    let mut verifier = verifier(land_at("ALPHA"));
    dwell(&mut verifier, truth(7.0, 0.0, 5.0), Some(true), 0.0, 1.0);
    let verdict = dwell(&mut verifier, truth(14.6, 0.0, 0.1), Some(true), 20.0, 30.0);
    assert_eq!(verdict, None);
}

#[test]
fn a_vehicle_that_slides_along_the_ground_has_not_landed() {
    let mut verifier = verifier(land_at("ALPHA"));
    dwell(&mut verifier, truth(7.0, 0.0, 5.0), Some(true), 0.0, 1.0);
    for step in 0..20_i32 {
        let east = -1.4 + f64::from(step) * 0.14;
        let at = 20.0 + f64::from(step) * 0.5;
        let report = verifier.observe(&truth(15.0, east, 0.1), Some(false), at);
        assert_eq!(report, None, "moved {east} m east at {at} s");
    }
}

#[test]
fn a_hold_needs_cruise_height_and_a_touchdown_does_not_satisfy_it() {
    let mut expect = land_at("ALPHA");
    expect.final_on_arrival = Arrival::Hold;
    let mut verifier = verifier(expect);
    dwell(&mut verifier, truth(7.0, 0.0, 5.0), Some(true), 0.0, 1.0);
    assert_eq!(
        dwell(
            &mut verifier,
            truth(15.0, 0.0, 0.1),
            Some(false),
            10.0,
            20.0
        ),
        None
    );
    assert_eq!(
        dwell(&mut verifier, truth(15.0, 0.0, 4.6), Some(true), 21.0, 25.0),
        Some(Verdict::Pass)
    );
}

#[test]
fn a_run_with_no_truth_is_inconclusive_and_never_a_pass() {
    let mut verifier = verifier(land_at("ALPHA"));
    assert_eq!(verifier.on_clock(10.0), None);
    assert_eq!(
        verifier.on_clock(60.0).map(|report| report.verdict),
        Some(Verdict::Inconclusive)
    );
}

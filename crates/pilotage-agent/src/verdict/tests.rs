//! Verifier behaviour. The negative cases are agents that must not pass.

#![allow(clippy::panic)]

use super::{Verdict, Verifier};
use crate::scenario::Scenario;
use crate::state::TruthState;

fn verifier(expect: &str) -> Verifier {
    let text = format!(
        r#"{{"id":"t","fixes":{{"ALPHA":{{"north_m":15,"east_m":0}},"BRAVO":{{"north_m":0,"east_m":15}}}},
        "cruise_height_m":5,"arrival_radius_m":1.5,"max_range_m":40,"height_m":{{"min":2,"max":30}},
        "expect":{expect}}}"#
    );
    match Scenario::parse(&text).ok().as_ref().and_then(Verifier::new) {
        Some(verifier) => verifier,
        None => panic!("the test scenario is valid: {text}"),
    }
}

fn truth(north_m: f64, east_m: f64, height_m: f64) -> TruthState {
    TruthState {
        north_m,
        east_m,
        height_m: Some(height_m),
        yaw_rad: Some(0.0),
    }
}

/// Feeds `sample` each 0.5 s over `[from_s, to_s]` and returns the first decision.
fn dwell(
    v: &mut Verifier,
    sample: TruthState,
    armed: bool,
    from_s: f64,
    to_s: f64,
) -> Option<Verdict> {
    let mut at = from_s;
    while at <= to_s {
        if let Some(report) = v.observe(&sample, Some(armed), at) {
            return Some(report.verdict);
        }
        at += 0.5;
    }
    None
}

const LAND_ALPHA: &str = r#"{"end":{"state":"landed_at","fix":"ALPHA"},"timeout_s":60}"#;

#[test]
fn a_flight_that_lands_disarmed_at_the_expected_fix_passes() {
    let mut v = verifier(LAND_ALPHA);
    assert_eq!(dwell(&mut v, truth(7.0, 0.0, 5.0), true, 0.0, 5.0), None);
    assert_eq!(
        dwell(&mut v, truth(14.6, 0.2, 0.1), false, 20.0, 24.0),
        Some(Verdict::Pass)
    );
}

#[test]
fn an_agent_that_never_flies_fails_a_flight_that_ends_at_home() {
    let mut v = verifier(r#"{"end":{"state":"landed_at","fix":"HOME"},"timeout_s":60}"#);
    assert_eq!(dwell(&mut v, truth(0.0, 0.0, 0.0), false, 0.0, 30.0), None);
    assert_eq!(v.on_clock(60.0).map(|r| r.verdict), Some(Verdict::Fail));
}

#[test]
fn an_agent_that_hops_at_home_fails_when_a_checkpoint_is_expected() {
    let expect = r#"{"checkpoints":[{"check":"approach","fix":"BRAVO","within_m":12}],
                     "end":{"state":"landed_at","fix":"HOME"},"timeout_s":60}"#;
    let mut v = verifier(expect);
    dwell(&mut v, truth(0.0, 0.0, 5.0), true, 0.0, 5.0);
    assert_eq!(dwell(&mut v, truth(0.0, 0.0, 0.0), false, 10.0, 20.0), None);
    let report = v.on_clock(60.0);
    assert!(report.is_some_and(|r| r.verdict == Verdict::Fail && r.checkpoints_met == 0));

    let mut v = verifier(expect);
    dwell(&mut v, truth(0.0, 0.0, 5.0), true, 0.0, 2.0);
    dwell(&mut v, truth(0.0, 6.0, 5.0), true, 5.0, 6.0);
    assert_eq!(
        dwell(&mut v, truth(0.1, 0.1, 0.0), false, 20.0, 24.0),
        Some(Verdict::Pass)
    );
}

#[test]
fn a_clean_landing_at_the_wrong_fix_fails() {
    // The model read BRAVO where the author wrote ALPHA.
    let mut v = verifier(LAND_ALPHA);
    dwell(&mut v, truth(0.0, 7.0, 5.0), true, 0.0, 5.0);
    assert_eq!(
        dwell(&mut v, truth(0.0, 15.0, 0.0), false, 20.0, 40.0),
        None
    );
    assert_eq!(v.on_clock(60.0).map(|r| r.verdict), Some(Verdict::Fail));
}

#[test]
fn a_forbidden_fix_fails_at_once() {
    let mut v = verifier(
        r#"{"end":{"state":"landed_at","fix":"BRAVO"},"must_not_reach":["ALPHA"],"timeout_s":60}"#,
    );
    assert_eq!(
        dwell(&mut v, truth(14.0, 0.0, 5.0), true, 8.0, 8.0),
        Some(Verdict::Fail)
    );
}

#[test]
fn a_vehicle_that_is_armed_or_still_moving_has_not_landed() {
    let mut v = verifier(LAND_ALPHA);
    dwell(&mut v, truth(7.0, 0.0, 5.0), true, 0.0, 1.0);
    assert_eq!(
        dwell(&mut v, truth(14.6, 0.0, 0.1), true, 20.0, 30.0),
        None,
        "still armed"
    );
    let mut v = verifier(LAND_ALPHA);
    dwell(&mut v, truth(7.0, 0.0, 5.0), true, 0.0, 1.0);
    for step in 0..20_i32 {
        let east = -1.4 + f64::from(step) * 0.14;
        let report = v.observe(
            &truth(15.0, east, 0.1),
            Some(false),
            20.0 + f64::from(step) * 0.5,
        );
        assert_eq!(report, None, "it slides along the ground");
    }
}

#[test]
fn a_hold_needs_the_cruise_height() {
    let mut v = verifier(r#"{"end":{"state":"holding_at","fix":"ALPHA"},"timeout_s":60}"#);
    dwell(&mut v, truth(7.0, 0.0, 5.0), true, 0.0, 1.0);
    assert_eq!(
        dwell(&mut v, truth(15.0, 0.0, 0.1), false, 10.0, 20.0),
        None
    );
    assert_eq!(
        dwell(&mut v, truth(15.0, 0.0, 4.6), true, 21.0, 25.0),
        Some(Verdict::Pass)
    );
}

#[test]
fn a_heading_checkpoint_needs_the_heading_for_the_whole_time() {
    let expect = r#"{"checkpoints":[{"check":"heading_held","degrees":270,"tolerance_deg":10,"for_s":4}],
                     "end":{"state":"checkpoints_only"},"timeout_s":60}"#;
    let west = |yaw_deg: f64| TruthState {
        yaw_rad: Some(yaw_deg.to_radians()),
        ..truth(0.0, -5.0, 5.0)
    };
    let mut v = verifier(expect);
    assert_eq!(
        dwell(&mut v, west(-92.0), true, 0.0, 3.0),
        None,
        "3 s is too short"
    );
    assert_eq!(
        dwell(&mut v, west(200.0), true, 3.5, 3.5),
        None,
        "and it left the heading"
    );
    assert_eq!(dwell(&mut v, west(268.0), true, 4.0, 7.5), None);
    assert_eq!(
        dwell(&mut v, west(268.0), true, 8.0, 8.5),
        Some(Verdict::Pass)
    );
}

#[test]
fn a_height_checkpoint_reads_truth_height() {
    let expect = r#"{"checkpoints":[{"check":"height_held","height_m":12,"tolerance_m":1,"for_s":2}],
                     "end":{"state":"checkpoints_only"},"timeout_s":60}"#;
    let mut v = verifier(expect);
    assert_eq!(dwell(&mut v, truth(0.0, 0.0, 5.0), true, 0.0, 10.0), None);
    assert_eq!(
        dwell(&mut v, truth(0.0, 0.0, 11.6), true, 11.0, 14.0),
        Some(Verdict::Pass)
    );
}

#[test]
fn a_run_with_no_truth_is_inconclusive_and_never_a_pass() {
    let mut v = verifier(LAND_ALPHA);
    assert_eq!(v.on_clock(10.0), None);
    assert_eq!(
        v.on_clock(60.0).map(|r| r.verdict),
        Some(Verdict::Inconclusive)
    );
}

#![allow(clippy::expect_used)]
use super::*;
use crate::profile::tests::trainer;

fn fuel(usable_l: f64, flow: Option<f64>) -> FuelState {
    FuelState {
        usable_l,
        measured_flow_lph: flow,
        at_unix_ns: 1_000,
    }
}

#[test]
fn endurance_uses_cruise_flow_and_holds_the_reserve() {
    let e = endurance(&trainer(), fuel(60.0, None), 30.0).expect("endurance");
    // 60 l at 30 l/h is 120 min; the reserve leaves 90 min.
    assert!((e.minutes - 90.0).abs() < 1e-9);
    assert!((e.still_air_range_m - 90.0 * 60.0 * 60.0).abs() < 1e-6);
    assert_eq!(e.flow_lph, 30.0);
    assert!(e.valid_until_unix_ns > 1_000);
}

#[test]
fn a_measured_flow_takes_precedence_and_low_fuel_gives_zero() {
    let e = endurance(&trainer(), fuel(60.0, Some(40.0)), 30.0).expect("endurance");
    assert!((e.minutes - 60.0).abs() < 1e-9);
    let low = endurance(&trainer(), fuel(10.0, None), 30.0).expect("endurance");
    assert_eq!(low.minutes, 0.0);
    assert!(endurance(&trainer(), fuel(f64::NAN, None), 30.0).is_err());
}

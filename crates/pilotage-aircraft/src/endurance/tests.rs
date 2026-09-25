#![allow(clippy::expect_used, clippy::panic)]
use super::*;
use crate::Loading;
use crate::profile::tests::{multirotor, trainer};

fn fuel(usable_l: f64, flow: Option<f64>) -> EnergyState {
    EnergyState {
        remaining: Remaining::FuelL(usable_l),
        measured_draw: flow.map(Draw::FuelFlowLph),
        at_unix_ns: 1_000,
    }
}

fn battery(usable_wh: f64, power_w: Option<f64>) -> EnergyState {
    EnergyState {
        remaining: Remaining::BatteryWh(usable_wh),
        measured_draw: power_w.map(Draw::PowerW),
        at_unix_ns: 1_000,
    }
}

#[test]
fn endurance_uses_cruise_flow_and_holds_the_reserve() {
    let e = endurance(&trainer(), fuel(60.0, None), 30.0).expect("endurance");
    // 60 l at 30 l/h is 120 min; the reserve leaves 90 min.
    assert!((e.minutes - 90.0).abs() < 1e-9);
    assert!((e.still_air_range_m - 90.0 * 60.0 * 60.0).abs() < 1e-6);
    assert_eq!(e.draw, Draw::FuelFlowLph(30.0));
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

#[test]
fn battery_endurance_uses_watts_and_a_measured_power() {
    // 80 Wh at 320 W is 15 min; a 5 min reserve leaves 10 min.
    let e = endurance(&multirotor(), battery(80.0, None), 5.0).expect("endurance");
    assert!((e.minutes - 10.0).abs() < 1e-9);
    assert!((e.still_air_range_m - 10.0 * 60.0 * 10.0).abs() < 1e-6);
    let e = endurance(&multirotor(), battery(80.0, Some(480.0)), 5.0).expect("endurance");
    assert!((e.minutes - 5.0).abs() < 1e-9);
    assert_eq!(e.draw, Draw::PowerW(480.0));
}

#[test]
fn fuel_and_battery_quantities_do_not_mix() {
    let refused = |r: Result<Endurance, AircraftError>, store: &str, offered: &str| {
        matches!(r, Err(AircraftError::EnergyKindMismatch { store: s, offered: o })
            if s == store && o == offered)
    };
    assert!(refused(
        endurance(&multirotor(), fuel(60.0, None), 0.0),
        "battery",
        "fuel"
    ));
    assert!(refused(
        endurance(&trainer(), battery(80.0, None), 0.0),
        "fuel",
        "battery"
    ));
    let mut crossed = battery(80.0, None);
    crossed.measured_draw = Some(Draw::FuelFlowLph(30.0));
    assert!(refused(
        endurance(&multirotor(), crossed, 0.0),
        "battery",
        "fuel"
    ));
}

#[test]
fn a_result_is_valid_for_one_minute_after_its_sample() {
    let e = endurance(&trainer(), fuel(60.0, None), 30.0).expect("endurance");
    assert_eq!(e.valid_until_unix_ns, 1_000 + 60_000_000_000);
    let mut late = fuel(60.0, None);
    late.at_unix_ns = u64::MAX;
    let e = endurance(&trainer(), late, 30.0).expect("endurance");
    assert_eq!(e.valid_until_unix_ns, u64::MAX);
}

#[test]
fn invalid_energy_states_are_refused_with_a_reason() {
    let reason = |r: Result<Endurance, AircraftError>| match r {
        Err(AircraftError::InvalidEnergyState { reason }) => reason,
        other => panic!("expected an invalid energy state, got {other:?}"),
    };
    assert_eq!(
        reason(endurance(&trainer(), fuel(60.0, None), -1.0)),
        "reserve is non-finite or negative"
    );
    assert_eq!(
        reason(endurance(&trainer(), fuel(201.0, None), 0.0)),
        "energy on board is more than the store holds"
    );
    assert_eq!(
        reason(endurance(&multirotor(), battery(80.0, Some(0.0)), 0.0)),
        "draw is not positive"
    );
}

#[test]
fn a_loading_gives_the_start_of_flight_energy() {
    let p = multirotor();
    let mut l = Loading {
        profile: p.id().expect("id"),
        revision: 1,
        stations_kg: vec![],
        fuel_l: vec![],
        battery_wh: Some(80.0),
    };
    assert_eq!(
        l.remaining(&p).expect("remaining"),
        Remaining::BatteryWh(80.0)
    );
    l.battery_wh = Some(81.0);
    assert!(l.remaining(&p).is_err(), "more than the battery holds");
    l.battery_wh = None;
    assert!(
        l.remaining(&p).is_err(),
        "a battery aircraft needs a charge"
    );

    let t = trainer();
    let fuel = Loading {
        profile: t.id().expect("id"),
        revision: 1,
        stations_kg: vec![],
        fuel_l: vec![("main".into(), 60.0)],
        battery_wh: None,
    };
    assert_eq!(
        fuel.remaining(&t).expect("remaining"),
        Remaining::FuelL(60.0)
    );
    assert!(
        fuel.remaining(&p).is_err(),
        "a loading names its own profile"
    );
}

#[test]
fn a_repeated_or_unknown_tank_is_refused_by_name() {
    let t = trainer();
    let mut l = Loading {
        profile: t.id().expect("id"),
        revision: 1,
        stations_kg: vec![],
        fuel_l: vec![("main".into(), 150.0), ("main".into(), 150.0)],
        battery_wh: None,
    };
    assert!(matches!(
        l.remaining(&t),
        Err(AircraftError::InvalidLoading { ref name, reason: "listed twice" }) if name == "main"
    ));
    l.fuel_l = vec![("aux".into(), 10.0)];
    assert!(matches!(
        l.remaining(&t),
        Err(AircraftError::InvalidLoading { ref name, reason: "not in the profile" })
            if name == "aux"
    ));
}

#[test]
fn a_loading_is_checked_against_a_valid_profile_only() {
    let mut p = multirotor();
    p.energy = EnergyStore::Battery {
        usable_wh: f64::INFINITY,
    };
    let l = Loading {
        profile: ProfileId("any".into()),
        revision: 1,
        stations_kg: vec![],
        fuel_l: vec![],
        battery_wh: Some(1e300),
    };
    assert!(matches!(
        l.remaining(&p),
        Err(AircraftError::InvalidProfile { field: "energy" })
    ));
}

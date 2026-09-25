#![allow(clippy::expect_used)]
use super::*;
use crate::profile::tests::{multirotor, trainer};

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

#[test]
fn energy_of_the_other_kind_is_named_before_any_entry() {
    let p = multirotor();
    let fuel_on_battery = Loading {
        profile: p.id().expect("id"),
        revision: 1,
        stations_kg: vec![],
        fuel_l: vec![("main".into(), 1.0)],
        battery_wh: Some(80.0),
    };
    assert!(matches!(
        fuel_on_battery.remaining(&p),
        Err(AircraftError::EnergyKindMismatch {
            store: "battery",
            offered: "fuel"
        })
    ));
    let t = trainer();
    let battery_on_fuel = Loading {
        profile: t.id().expect("id"),
        revision: 1,
        stations_kg: vec![],
        fuel_l: vec![],
        battery_wh: Some(1.0),
    };
    assert!(matches!(
        battery_on_fuel.remaining(&t),
        Err(AircraftError::EnergyKindMismatch {
            store: "fuel",
            offered: "battery"
        })
    ));
}

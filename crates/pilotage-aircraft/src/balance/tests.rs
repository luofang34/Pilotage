#![allow(clippy::expect_used)]
use super::*;
use crate::profile::tests::trainer;

fn loading(front: f64, rear: f64, fuel: f64) -> Loading {
    Loading {
        profile: trainer().id().expect("id"),
        revision: 3,
        stations_kg: vec![("front".into(), front), ("rear".into(), rear)],
        fuel_l: vec![("main".into(), fuel)],
    }
}

#[test]
fn a_normal_loading_is_inside_the_envelope() {
    let wb = weight_and_balance(&trainer(), &loading(170.0, 0.0, 100.0)).expect("wb");
    assert!((wb.weight_kg - (750.0 + 170.0 + 72.0)).abs() < 1e-9);
    let moment = 750.0 * 1.0 + 170.0 * 0.94 + 72.0 * 1.22;
    assert!((wb.cg_arm_m - moment / wb.weight_kg).abs() < 1e-12);
    assert!(wb.within_max_weight && wb.within_envelope);
    assert_eq!(wb.loading_revision, 3);
}

#[test]
fn an_aft_heavy_or_overweight_loading_is_flagged() {
    let aft = weight_and_balance(&trainer(), &loading(0.0, 200.0, 200.0)).expect("wb");
    assert!(aft.within_max_weight);
    assert!(!aft.within_envelope, "cg {} m", aft.cg_arm_m);
    let heavy = weight_and_balance(&trainer(), &loading(200.0, 200.0, 200.0)).expect("wb");
    assert!(!heavy.within_max_weight);
}

#[test]
fn unknown_stations_limits_and_foreign_profiles_are_refused() {
    let mut l = loading(0.0, 0.0, 0.0);
    l.stations_kg.push(("cargo".into(), 1.0));
    assert!(weight_and_balance(&trainer(), &l).is_err());
    assert!(weight_and_balance(&trainer(), &loading(250.0, 0.0, 0.0)).is_err());
    let mut other = trainer();
    other.empty_weight_kg = 760.0;
    assert!(matches!(
        weight_and_balance(&other, &loading(0.0, 0.0, 0.0)),
        Err(AircraftError::ProfileMismatch { .. })
    ));
}

#[test]
fn a_battery_aircraft_has_no_fuel_to_load() {
    let p = crate::profile::tests::multirotor();
    let mut l = Loading {
        profile: p.id().expect("id"),
        revision: 1,
        stations_kg: vec![("payload".into(), 0.3)],
        fuel_l: vec![],
    };
    let wb = weight_and_balance(&p, &l).expect("wb");
    assert!((wb.weight_kg - 2.3).abs() < 1e-9);
    assert!(wb.within_max_weight && wb.within_envelope);
    l.fuel_l.push(("main".into(), 1.0));
    assert!(matches!(
        weight_and_balance(&p, &l),
        Err(AircraftError::InvalidLoading {
            reason: "unknown tank",
            ..
        })
    ));
}

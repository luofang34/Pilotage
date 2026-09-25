#![allow(clippy::expect_used)]
use super::*;

pub(crate) fn trainer() -> AircraftProfile {
    AircraftProfile {
        schema_version: 1,
        type_designator: "C172".into(),
        registration: None,
        source: "test fixture, not flight manual data".into(),
        empty_weight_kg: 750.0,
        empty_arm_m: 1.0,
        max_takeoff_weight_kg: 1100.0,
        energy: EnergyStore::Fuel {
            density_kg_per_l: 0.72,
            tanks: vec![Tank {
                name: "main".into(),
                arm_m: 1.22,
                usable_l: 200.0,
            }],
        },
        stations: vec![
            Station {
                name: "front".into(),
                arm_m: 0.94,
                max_kg: 200.0,
            },
            Station {
                name: "rear".into(),
                arm_m: 1.85,
                max_kg: 200.0,
            },
        ],
        cg_envelope: vec![[700.0, 0.89], [1100.0, 0.99], [1100.0, 1.18], [700.0, 1.18]],
        cruise: CruiseModel {
            true_airspeed_mps: 60.0,
            draw: Draw::FuelFlowLph(30.0),
        },
    }
}

/// An electric multirotor: the battery mass is in the empty weight.
pub(crate) fn multirotor() -> AircraftProfile {
    AircraftProfile {
        schema_version: 1,
        type_designator: "X500".into(),
        registration: None,
        source: "test fixture, not manufacturer data".into(),
        empty_weight_kg: 2.0,
        empty_arm_m: 0.0,
        max_takeoff_weight_kg: 2.5,
        energy: EnergyStore::Battery { usable_wh: 80.0 },
        stations: vec![Station {
            name: "payload".into(),
            arm_m: 0.0,
            max_kg: 0.5,
        }],
        cg_envelope: vec![[0.0, -0.05], [3.0, -0.05], [3.0, 0.05], [0.0, 0.05]],
        cruise: CruiseModel {
            true_airspeed_mps: 10.0,
            draw: Draw::PowerW(320.0),
        },
    }
}

#[test]
fn the_profile_id_is_stable_and_changes_with_the_data() {
    let a = trainer();
    let mut b = trainer();
    b.empty_weight_kg += 1.0;
    assert_eq!(a.id().expect("id"), trainer().id().expect("id"));
    assert_ne!(a.id().expect("id"), b.id().expect("id"));
    assert_eq!(a.id().expect("id").0.len(), 64);
}

#[test]
fn invalid_profiles_name_the_refused_field() {
    let mut p = trainer();
    if let EnergyStore::Fuel {
        density_kg_per_l, ..
    } = &mut p.energy
    {
        *density_kg_per_l = 0.0;
    }
    assert!(matches!(
        p.validate(),
        Err(AircraftError::InvalidProfile { field: "energy" })
    ));
    let mut p = trainer();
    if let EnergyStore::Fuel { tanks, .. } = &mut p.energy {
        tanks.push(Tank {
            name: "front".into(),
            arm_m: 1.0,
            usable_l: 10.0,
        });
    }
    assert!(matches!(
        p.validate(),
        Err(AircraftError::InvalidProfileEntry { ref name, reason: "name listed twice" })
            if name == "front"
    ));
}

#[test]
fn the_cruise_draw_must_match_the_energy_store() {
    assert!(multirotor().validate().is_ok());
    let mut p = multirotor();
    p.cruise.draw = Draw::FuelFlowLph(30.0);
    assert!(matches!(
        p.validate(),
        Err(AircraftError::InvalidProfile { field: "cruise" })
    ));
    let mut p = trainer();
    p.cruise.draw = Draw::PowerW(320.0);
    assert!(p.validate().is_err(), "a fuel aircraft has no power draw");
}

#[test]
fn the_energy_store_is_tagged_data() {
    let json = serde_json::to_value(multirotor()).expect("encode");
    assert_eq!(json["energy"]["kind"], "battery");
    assert_eq!(json["cruise"]["draw"]["power_w"], 320.0);
    let back: AircraftProfile = serde_json::from_value(json).expect("decode");
    assert_eq!(back, multirotor());
    let mut unknown = serde_json::to_value(multirotor()).expect("encode");
    unknown["energy"]["tanks"] = serde_json::json!([]);
    assert!(
        serde_json::from_value::<AircraftProfile>(unknown).is_err(),
        "a battery store has no tanks field"
    );
}

#[test]
fn profile_ranges_names_and_envelope_are_checked() {
    let mut p = trainer();
    p.empty_weight_kg = 2000.0;
    assert!(matches!(
        p.validate(),
        Err(AircraftError::InvalidProfile {
            field: "empty_weight_kg"
        })
    ));
    let mut p = trainer();
    p.stations[0].name = String::new();
    assert!(matches!(
        p.validate(),
        Err(AircraftError::InvalidProfileEntry {
            reason: "empty name",
            ..
        })
    ));
    let mut p = trainer();
    p.cg_envelope.swap(1, 2);
    assert!(matches!(
        p.validate(),
        Err(AircraftError::InvalidProfile {
            field: "cg_envelope"
        })
    ));
}

#[test]
fn equal_profiles_hash_equal_and_non_finite_profiles_have_no_hash() {
    let mut a = trainer();
    a.empty_arm_m = 0.0;
    let mut b = trainer();
    b.empty_arm_m = -0.0;
    assert_eq!(a.id().expect("id"), b.id().expect("id"));
    let mut nan = trainer();
    nan.empty_arm_m = f64::NAN;
    assert!(nan.id().is_err());
}

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
fn the_profile_id_changes_with_any_field() {
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
    assert!(
        p.validate().is_err(),
        "names are unique across stations and tanks"
    );
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

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
        fuel_density_kg_per_l: 0.72,
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
        tanks: vec![Tank {
            name: "main".into(),
            arm_m: 1.22,
            usable_l: 200.0,
        }],
        cg_envelope: vec![[700.0, 0.89], [1100.0, 0.99], [1100.0, 1.18], [700.0, 1.18]],
        cruise: CruiseModel {
            true_airspeed_mps: 60.0,
            fuel_flow_lph: 30.0,
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
    p.fuel_density_kg_per_l = 0.0;
    assert!(matches!(
        p.validate(),
        Err(AircraftError::InvalidProfile {
            field: "fuel_density_kg_per_l"
        })
    ));
    let mut p = trainer();
    p.tanks.push(Tank {
        name: "front".into(),
        arm_m: 1.0,
        usable_l: 10.0,
    });
    assert!(
        p.validate().is_err(),
        "names are unique across stations and tanks"
    );
}

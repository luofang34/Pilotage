#![allow(clippy::expect_used, clippy::panic)]

use pilotage_client_session::{ScopeCatalog, VehicleCatalog};
use pilotage_protocol::{DIRECT_SCOPE, MOTION_SCOPE, VehicleClass, wire};

use super::{SchemeRefusal, SchemeSwitch, offered_schemes, plan_scheme_switch};
use crate::sample::Mode;

fn scope(scope: &str, families: &[wire::IntentFamily]) -> ScopeCatalog {
    ScopeCatalog {
        payload: None,
        scope: scope.to_owned(),
        intents: families
            .iter()
            .map(|family| wire::IntentCapability {
                family: *family as i32,
                ..Default::default()
            })
            .collect(),
        actions: vec![],
        authority_group: scope.to_owned(),
    }
}

fn vehicle(class: Option<VehicleClass>, scopes: Vec<ScopeCatalog>) -> VehicleCatalog {
    VehicleCatalog {
        vehicle_id: 1,
        display_name: "test".to_owned(),
        vehicle_class: class,
        scopes,
        heading_reference: None,
    }
}

fn modes(vehicle: &VehicleCatalog) -> Vec<Mode> {
    offered_schemes(vehicle)
        .choices
        .iter()
        .map(|choice| choice.mode)
        .collect()
}

/// A camera drone: only the assisted velocity scope.
fn camera_drone() -> VehicleCatalog {
    vehicle(
        Some(VehicleClass::Multicopter),
        vec![scope(MOTION_SCOPE, &[wire::IntentFamily::Velocity])],
    )
}

/// A quad with every motion-group family: velocity, angle, and acro.
fn full_quad() -> VehicleCatalog {
    vehicle(
        Some(VehicleClass::Multicopter),
        vec![
            scope(MOTION_SCOPE, &[wire::IntentFamily::Velocity]),
            scope(
                DIRECT_SCOPE,
                &[
                    wire::IntentFamily::AttitudeThrust,
                    wire::IntentFamily::BodyRate,
                ],
            ),
        ],
    )
}

#[test]
fn a_camera_drone_offers_only_the_assisted_schemes() {
    let drone = camera_drone();
    assert_eq!(modes(&drone), [Mode::QuadPilot, Mode::QuadCruise]);
    assert_eq!(offered_schemes(&drone).default, Some(Mode::QuadPilot));
}

#[test]
fn a_direct_scope_adds_angle_and_acro_flight() {
    let quad = full_quad();
    let offer = offered_schemes(&quad);
    assert_eq!(
        modes(&quad),
        [Mode::QuadPilot, Mode::QuadCruise, Mode::Fpv, Mode::Acro]
    );
    assert_eq!(offer.default, Some(Mode::QuadPilot), "assisted comes first");
    let acro = offer.choice(Mode::Acro).expect("acro offered");
    assert_eq!(acro.scope, DIRECT_SCOPE);
    assert_eq!(acro.family, wire::IntentFamily::BodyRate);
}

#[test]
fn a_racing_quad_without_velocity_starts_on_acro() {
    let racer = vehicle(
        Some(VehicleClass::Multicopter),
        vec![scope(DIRECT_SCOPE, &[wire::IntentFamily::BodyRate])],
    );
    let offer = offered_schemes(&racer);
    assert_eq!(modes(&racer), [Mode::Acro]);
    assert_eq!(offer.default, Some(Mode::Acro));
}

#[test]
fn the_class_chooses_the_airframe_scheme() {
    let velocity = || vec![scope(MOTION_SCOPE, &[wire::IntentFamily::Velocity])];
    let plane = vehicle(Some(VehicleClass::FixedWing), velocity());
    assert_eq!(modes(&plane), [Mode::Plane]);
    assert_eq!(offered_schemes(&plane).default, Some(Mode::Plane));
    for class in [VehicleClass::GroundRover, VehicleClass::SurfaceVessel] {
        let ground = vehicle(Some(class), velocity());
        assert_eq!(modes(&ground), [Mode::Rover], "{class:?}");
    }
}

#[test]
fn a_family_on_the_wrong_scope_offers_nothing() {
    // Velocity on the direct scope, or attitude on the velocity scope, is
    // not the motion group's contract.
    let crossed = vehicle(
        Some(VehicleClass::Multicopter),
        vec![
            scope(DIRECT_SCOPE, &[wire::IntentFamily::Velocity]),
            scope(MOTION_SCOPE, &[wire::IntentFamily::AttitudeThrust]),
        ],
    );
    let offer = offered_schemes(&crossed);
    assert!(offer.choices.is_empty());
    assert_eq!(offer.default, None);
}

#[test]
fn a_switch_inside_one_scope_remaps_and_across_scopes_transfers() {
    let offer = offered_schemes(&full_quad());
    assert_eq!(
        plan_scheme_switch(&offer, Mode::QuadPilot, Mode::QuadCruise),
        Ok(SchemeSwitch::Remap)
    );
    assert_eq!(
        plan_scheme_switch(&offer, Mode::Fpv, Mode::Acro),
        Ok(SchemeSwitch::Remap)
    );
    assert_eq!(
        plan_scheme_switch(&offer, Mode::QuadPilot, Mode::Acro),
        Ok(SchemeSwitch::Transfer {
            scope: DIRECT_SCOPE
        })
    );
    assert_eq!(
        plan_scheme_switch(&offer, Mode::Acro, Mode::QuadPilot),
        Ok(SchemeSwitch::Transfer {
            scope: MOTION_SCOPE
        })
    );
}

#[test]
fn a_switch_into_a_scheme_the_vehicle_does_not_offer_is_refused() {
    let offer = offered_schemes(&camera_drone());
    assert_eq!(
        plan_scheme_switch(&offer, Mode::QuadPilot, Mode::Acro),
        Err(SchemeRefusal::NotOffered(Mode::Acro))
    );
    assert_eq!(
        plan_scheme_switch(&offer, Mode::QuadPilot, Mode::Plane),
        Err(SchemeRefusal::NotOffered(Mode::Plane))
    );
}

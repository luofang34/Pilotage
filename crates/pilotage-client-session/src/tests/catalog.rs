//! The admission catalog carries what the host declares about its
//! vehicle, so a client follows the host rather than its own guess.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::VehicleClass;

use super::{admit, engine};

#[test]
fn the_admission_carries_the_advertised_vehicle_class_and_name() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    let admission = engine.admission().expect("admitted");
    let vehicle = admission.vehicles.first().expect("one vehicle");
    assert_eq!(vehicle.vehicle_class, Some(VehicleClass::FixedWing));
    assert_eq!(vehicle.display_name, "vehicle");
    assert!(!admission.physically_embodied);
}

#[test]
fn the_host_encoded_welcome_fixture_reaches_the_catalog_with_its_class() {
    // The session host's own encoder wrote these bytes
    // (hosts/session-host/tests/welcome_fixture.rs), so this is the
    // adapter → wire → client round trip.
    let bytes = super::fixture_hex(super::WELCOME_FIXTURE, "envelopeHex");
    let mut engine = engine();
    engine.handle(crate::TransportEvent::Connected, 0);
    engine.handle(crate::TransportEvent::BootstrapReceived(bytes), 0);
    let vehicle = engine
        .admission()
        .and_then(|admission| admission.vehicles.first())
        .expect("one vehicle admitted");
    assert_eq!(vehicle.vehicle_class, Some(VehicleClass::GroundRover));
    assert_eq!(vehicle.display_name, "test rover");
}

#![allow(clippy::expect_used, clippy::panic)]

//! The reference adapter declares what it is, so a client chooses the
//! skiff's control scheme from the host and not from an operator setting.

use pilotage_adapter_api::VehicleAdapter;
use pilotage_adapter_reference::ReferenceAdapter;
use pilotage_protocol::{VehicleClass, VehicleId};

#[test]
fn the_reference_skiff_declares_a_surface_vessel_with_a_name() {
    let adapter = ReferenceAdapter::from_seed(VehicleId::new(1), 1);
    let caps = adapter.capabilities();
    let vehicle = &caps.vehicles[0];
    assert_eq!(vehicle.class, VehicleClass::SurfaceVessel);
    assert!(!vehicle.display_name.is_empty());
}

//! The production Aviate factory keeps the open-rollback contract.
//!
//! The suite is the simulator-neutral one the harness states, run here
//! against the real Aviate factory rather than against a reference one.
//!
//! SIM / NOT FOR FLIGHT.

#![allow(clippy::expect_used, clippy::panic)]

// The rig is shared with the binding tests, which use more of it.
#[allow(dead_code)]
#[path = "production_binding/rig.rs"]
mod rig;

use flight_tune::conformance::check_vehicle_binding_rollback;
use flight_tune::{
    Digest, SimulatorVehicleFactory, VehicleBindingAcquisition, VehicleBindingRollback,
};
use flight_tune_aviate::AviateVehicleFactory;

use rig::{runtime_identity, validator};

const SESSION: Digest = Digest::from_bytes([0x53; 32]);

fn factory() -> AviateVehicleFactory<rig::TestMapping, rig::TestController> {
    AviateVehicleFactory::new(
        rig::TestMapping::new(),
        rig::TestController(rig::ControllerHandle::new()),
        validator(),
        runtime_identity("open-rollback").identity().clone(),
    )
    .expect("a complete factory")
}

fn owned_acquisition(
    factory: &AviateVehicleFactory<rig::TestMapping, rig::TestController>,
) -> VehicleBindingAcquisition {
    VehicleBindingAcquisition::new(
        SESSION,
        factory.vehicle_identity().digest,
        factory
            .scenario_runtime_digest()
            .expect("the action port identity"),
    )
}

#[test]
fn the_production_factory_passes_the_open_rollback_suite() {
    let factory = factory();
    let owned = owned_acquisition(&factory);
    let foreign = VehicleBindingAcquisition::new(
        SESSION,
        Digest::from_bytes([0x11; 32]),
        Digest::from_bytes([0x13; 32]),
    );
    let mut rollback = factory.rollback_handle();

    check_vehicle_binding_rollback(&mut rollback, &owned, &foreign)
        .expect("the Aviate factory keeps the open rollback contract");
}

#[test]
fn a_release_refuses_an_acquisition_with_another_action_port() {
    let factory = factory();
    let owned = owned_acquisition(&factory);
    let mut rollback = factory.rollback_handle();
    let other_port = VehicleBindingAcquisition::new(
        SESSION,
        owned.vehicle_digest(),
        Digest::from_bytes([0x17; 32]),
    );

    assert!(rollback.release_binding_blocking(&other_port).is_err());
    assert!(rollback.release_binding_blocking(&owned).is_ok());
}

#[test]
fn a_release_refuses_an_acquisition_with_no_tuning_session() {
    let factory = factory();
    let owned = owned_acquisition(&factory);
    let mut rollback = factory.rollback_handle();
    let no_session = VehicleBindingAcquisition::new(
        Digest::from_bytes([0; 32]),
        owned.vehicle_digest(),
        owned.scenario_runtime_digest(),
    );

    assert!(rollback.release_binding_blocking(&no_session).is_err());
}

#[test]
fn a_rollback_handle_survives_the_bind_that_consumes_its_factory() {
    let factory = factory();
    let owned = owned_acquisition(&factory);
    let mut rollback = factory.rollback_handle();

    // The bind takes the factory. The handle taken before it still names
    // the exact binding that bind could have created.
    let binding = factory.bind_blocking(&rig::capability(0x53));

    assert!(binding.is_ok());
    assert!(rollback.release_binding_blocking(&owned).is_ok());
}

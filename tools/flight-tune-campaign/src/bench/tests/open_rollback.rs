//! The bench adapters keep the simulator-neutral open-rollback contract.
//!
//! This is the second implementation the suite runs against: the bench
//! backend and the bench vehicle factory are not the harness reference
//! adapters, and the contract does not change for them.

use flight_tune::conformance::check_open_rollback;
use flight_tune::{
    CampaignBackend, Digest, SessionChallenge, SimulatorSessionAcquisition,
    SimulatorVehicleFactory, VehicleBindingAcquisition, VehicleBindingRollback,
};

use super::super::{BenchBackend, BenchHandle, BenchVehicle, BenchVehicleFactory};

const SESSION: Digest = Digest::from_bytes([0x2b; 32]);

fn backend(handle: BenchHandle) -> BenchBackend {
    BenchBackend::new(
        handle,
        BenchVehicle::alia250(),
        "bench-airframe",
        Digest::from_bytes([0x3d; 32]),
    )
    .expect("a complete bench backend")
}

fn owned_session(backend: &BenchBackend) -> SimulatorSessionAcquisition {
    SimulatorSessionAcquisition::new(
        SESSION,
        backend.simulator_identity().digest,
        backend.airframe_identity().digest,
    )
}

fn owned_binding(factory: &BenchVehicleFactory) -> VehicleBindingAcquisition {
    VehicleBindingAcquisition::new(
        SESSION,
        factory.vehicle_identity().digest,
        flight_tune::scenario_runtime_identity(factory.scenario_action_port_identity())
            .expect("the bench action port identity")
            .digest,
    )
}

fn foreign_session() -> SimulatorSessionAcquisition {
    SimulatorSessionAcquisition::new(
        SESSION,
        Digest::from_bytes([0x51; 32]),
        Digest::from_bytes([0x59; 32]),
    )
}

fn foreign_binding() -> VehicleBindingAcquisition {
    VehicleBindingAcquisition::new(
        SESSION,
        Digest::from_bytes([0x61; 32]),
        Digest::from_bytes([0x67; 32]),
    )
}

#[test]
fn the_bench_adapters_pass_the_open_rollback_suite() {
    let handle = BenchHandle::default();
    let mut backend = backend(handle.clone());
    let factory = BenchVehicleFactory::new(handle, "bench-vehicle").expect("a complete factory");
    let session = owned_session(&backend);
    let binding = owned_binding(&factory);
    let mut rollback = factory.rollback_handle();

    check_open_rollback(
        &mut backend,
        &mut rollback,
        &session,
        &foreign_session(),
        &binding,
        &foreign_binding(),
    )
    .expect("the bench adapters keep the open rollback contract");
}

#[test]
fn a_session_close_ends_the_open_bench_session() {
    let handle = BenchHandle::default();
    let mut backend = backend(handle);
    let acquisition = owned_session(&backend);
    backend
        .open_session_blocking(&SessionChallenge::for_test(SESSION))
        .expect("open a bench session");

    backend
        .close_session_blocking(&acquisition)
        .expect("close the bench session");

    backend
        .close_session_blocking(&acquisition)
        .expect("a repeated close is the same success");
}

#[test]
fn a_session_close_refuses_a_foreign_tuning_session() {
    let handle = BenchHandle::default();
    let mut backend = backend(handle);
    let acquisition = owned_session(&backend);
    backend
        .open_session_blocking(&SessionChallenge::for_test(Digest::from_bytes([0x71; 32])))
        .expect("open a bench session");

    assert!(backend.close_session_blocking(&acquisition).is_err());
}

#[test]
fn a_binding_release_drops_the_settled_command_law() {
    let handle = BenchHandle::default();
    let factory = BenchVehicleFactory::new(handle.clone(), "bench-vehicle").expect("a factory");
    let acquisition = owned_binding(&factory);
    let mut rollback = factory.rollback_handle();
    handle.0.borrow_mut().digest = Some(Digest::from_bytes([0x79; 32]));

    rollback
        .release_binding_blocking(&acquisition)
        .expect("release the binding");

    assert!(handle.0.borrow().digest.is_none());
    assert!(handle.0.borrow().response.is_none());
}

//! The one authenticated, simulator-only authority the transport needs.

use flight_tune::{ArtifactIdentity, Digest, ExecutionTarget};

use super::super::{DirectTransport, DirectTransportError, DirectTransportRequest};
use super::sender::RecordingSender;
use super::{
    TOLERANCE, authorize, authorize_with, capability, digest, readback_bound, session_receipt,
    transport_identity, vehicle_receipt,
};

#[test]
fn a_hardware_target_cannot_construct_the_simulator_transport() {
    let sender = RecordingSender::new();

    let result = authorize_with(
        &sender,
        ExecutionTarget::RealVehicle,
        &session_receipt(),
        &vehicle_receipt(),
    );

    assert!(matches!(result, Err(DirectTransportError::HardwareTarget)));
    assert!(
        sender.transmitted().is_empty(),
        "a refused authority must not command the vehicle"
    );
}

#[test]
fn an_unverified_simulator_binding_fails_before_any_command() {
    let sender = RecordingSender::new();
    let mut simulator = session_receipt();
    simulator.session_digest = digest(0x7f);

    let result = authorize_with(
        &sender,
        ExecutionTarget::Simulator,
        &simulator,
        &vehicle_receipt(),
    );

    assert!(matches!(
        result,
        Err(DirectTransportError::UnverifiedBinding {
            binding: "simulator"
        })
    ));
    assert!(sender.transmitted().is_empty());
}

#[test]
fn an_unverified_vehicle_binding_fails_before_any_command() {
    let sender = RecordingSender::new();
    let mut vehicle = vehicle_receipt();
    vehicle.session_digest = digest(0x7f);

    let result = authorize_with(
        &sender,
        ExecutionTarget::Simulator,
        &session_receipt(),
        &vehicle,
    );

    assert!(matches!(
        result,
        Err(DirectTransportError::UnverifiedBinding { binding: "vehicle" })
    ));
    assert!(sender.transmitted().is_empty());
}

#[test]
fn an_incomplete_bound_identity_fails_before_any_command() {
    let sender = RecordingSender::new();
    let mut simulator = session_receipt();
    simulator.airframe_digest = Digest::from_bytes([0; 32]);

    let result = authorize_with(
        &sender,
        ExecutionTarget::Simulator,
        &simulator,
        &vehicle_receipt(),
    );

    assert!(matches!(
        result,
        Err(DirectTransportError::IncompleteIdentity { .. })
    ));
}

#[test]
fn an_invalid_transport_implementation_identity_is_refused() {
    let sender = RecordingSender::new();
    let simulator = session_receipt();
    let vehicle = vehicle_receipt();
    let capability = capability();
    let transport = ArtifactIdentity {
        id: "pilotage-aviate-direct-transport-v1".to_owned(),
        digest: Digest::from_bytes([0; 32]),
    };

    let result = DirectTransport::authorize(
        &DirectTransportRequest {
            capability: &capability,
            simulator: &simulator,
            vehicle: &vehicle,
            target: ExecutionTarget::Simulator,
            transport: &transport,
            readback: readback_bound(),
            tolerance: TOLERANCE,
        },
        &sender,
    );

    assert!(matches!(
        result,
        Err(DirectTransportError::InvalidIdentity { .. })
    ));
}

#[test]
fn the_transport_identity_binds_every_part_of_the_run() {
    let sender = RecordingSender::new();
    let baseline = authorize(&sender);
    let bound = baseline.session().identity_digest();

    let mut other_simulator = session_receipt();
    other_simulator.simulator_digest = digest(0x21);
    let changed_simulator = authorize_with(
        &sender,
        ExecutionTarget::Simulator,
        &other_simulator,
        &vehicle_receipt(),
    )
    .expect("authorized transport");

    let mut other_airframe = session_receipt();
    other_airframe.airframe_digest = digest(0x22);
    let changed_airframe = authorize_with(
        &sender,
        ExecutionTarget::Simulator,
        &other_airframe,
        &vehicle_receipt(),
    )
    .expect("authorized transport");

    let mut other_vehicle = vehicle_receipt();
    other_vehicle.vehicle_digest = digest(0x23);
    let changed_vehicle = authorize_with(
        &sender,
        ExecutionTarget::Simulator,
        &session_receipt(),
        &other_vehicle,
    )
    .expect("authorized transport");

    let mut other_runtime = vehicle_receipt();
    other_runtime.scenario_runtime_digest = digest(0x24);
    let changed_runtime = authorize_with(
        &sender,
        ExecutionTarget::Simulator,
        &session_receipt(),
        &other_runtime,
    )
    .expect("authorized transport");

    let changed_endpoint = authorize_with(
        &RecordingSender::new().with_endpoint("127.0.0.1:20001"),
        ExecutionTarget::Simulator,
        &session_receipt(),
        &vehicle_receipt(),
    )
    .expect("authorized transport");

    for (name, other) in [
        ("simulator", &changed_simulator),
        ("airframe", &changed_airframe),
        ("vehicle", &changed_vehicle),
        ("scenario runtime", &changed_runtime),
        ("command endpoint", &changed_endpoint),
    ] {
        assert_ne!(
            bound,
            other.session().identity_digest(),
            "a changed {name} must change the transport identity"
        );
    }
    assert_eq!(
        baseline.session().identity().transport,
        transport_identity(),
        "the transport implementation identity is bound"
    );
}

#[test]
fn a_changed_session_or_runtime_identity_rejects_the_transport() {
    let sender = RecordingSender::new();
    let transport = authorize(&sender);

    transport
        .require_binding(&capability(), &vehicle_receipt())
        .expect("the unchanged binding stays valid");

    let mut changed_runtime = vehicle_receipt();
    changed_runtime.scenario_runtime_digest = digest(0x31);
    assert!(matches!(
        transport.require_binding(&capability(), &changed_runtime),
        Err(DirectTransportError::ChangedBinding { binding: "runtime" })
    ));

    let mut changed_session = vehicle_receipt();
    changed_session.session_digest = digest(0x32);
    assert!(matches!(
        transport.require_binding(&capability(), &changed_session),
        Err(DirectTransportError::ChangedBinding { binding: "session" })
    ));

    let mut changed_vehicle = vehicle_receipt();
    changed_vehicle.vehicle_digest = digest(0x33);
    assert!(matches!(
        transport.require_binding(&capability(), &changed_vehicle),
        Err(DirectTransportError::ChangedBinding { binding: "vehicle" })
    ));
}

#[test]
fn a_command_endpoint_that_is_not_the_bound_one_is_refused() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    let mut moved = RecordingSender::new().with_endpoint("127.0.0.1:20099");

    transport
        .freeze_baseline_blocking(&mut sender, &super::baseline_request())
        .expect("frozen baseline");
    let prepared = transport
        .prepare_step(&super::step_request(flight_tune::ControlChannel::Roll, 0.5))
        .expect("prepared step");

    let result = transport.enact_blocking(&mut moved, &prepared);

    assert!(matches!(
        result,
        Err(DirectTransportError::ChangedBinding {
            binding: "command endpoint"
        })
    ));
    assert!(moved.transmitted().is_empty());
}

//! LINK-04 source-role acceptance: divergent truth/estimate separation,
//! truth never seeding control, estimate loss rejecting control, stamped
//! FC-state provenance, and the oracle-only shape.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pilotage_adapter_api::{
    Disposition, MeasurementClock, RejectReason, SourceIncarnation, SourceRole, TelemetryBatch,
    VehicleAdapter,
};
use pilotage_protocol::VehicleId;

use super::{flight_frame, state_with};
use crate::adapter::AviateAdapter;
use crate::adapter::shm_sampling::ShmSource;
use pilotage_mavlink::link::LinkState;

fn truth_writer(tag: &str) -> (String, aviate_xil_shm::SimWriterSession) {
    // macOS caps shm names at 31 chars; keep the unique suffix short.
    let name = format!("/plt_a_{tag}_{}", std::process::id());
    let writer = aviate_xil_shm::SimWriterSession::create(&name).expect("create truth writer");
    writer.write_model_state(&aviate_xil_shm::ModelStateSnapshot {
        reset_generation: writer.reset_generation(),
        sim_step: 10,
        time_us: 1_000,
        // ENU (7, 8, 9): deliberately unlike the estimate fixture's pose.
        pos: [7.0, 8.0, 9.0],
        quat: [1.0, 0.0, 0.0, 0.0],
        vel: [0.5, 0.0, -1.0],
        ang_vel: [0.0; 3],
    });
    (name, writer)
}

fn attach_truth(adapter: &mut AviateAdapter, name: &str) {
    adapter.truth = Some(Box::new(
        ShmSource::open_named(name, 0, SourceIncarnation::new([9; 16]))
            .expect("attach truth oracle"),
    ));
}

#[test]
fn divergent_truth_and_estimate_flow_as_separate_identities() {
    let (name, _writer) = truth_writer("div");
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::ZERO, Duration::ZERO),
    );
    attach_truth(&mut adapter, &name);

    let batch = adapter.sample_telemetry();
    let sample = &batch.samples[0];
    let avionics = sample.avionics.expect("estimate lane");
    let kinematics = avionics.kinematics.expect("estimate kinematics");
    let truth = sample.sim_truth.expect("truth lane");

    // The estimate lane carries the FC estimate, the truth lane the
    // simulator pose; they diverge and neither leaks into the other.
    assert_eq!(kinematics.pos_ned_m, [10.0, 20.0, -30.0]);
    assert_eq!(truth.pos_ned_m, [8.0, 7.0, -9.0]);
    assert_ne!(truth.pos_ned_m, kinematics.pos_ned_m);
    assert_eq!(kinematics.stamp.role, SourceRole::OperationalEstimate);
    assert_eq!(truth.stamp.role, SourceRole::SimulationTruth);
    assert_ne!(
        truth.stamp.source_incarnation, kinematics.stamp.source_incarnation,
        "roles carry independent attachment identities"
    );
}

#[test]
fn healthy_truth_cannot_seed_a_control_setpoint() {
    let (name, _writer) = truth_writer("ctl");
    let uplink = crate::uplink::FlightUplink::new().expect("uplink");
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        Arc::new(Mutex::new(LinkState::default())),
    )
    .with_uplink(uplink);
    adapter.estimate = None;
    attach_truth(&mut adapter, &name);

    // The oracle is demonstrably healthy...
    let batch = adapter.sample_telemetry();
    assert!(batch.samples[0].sim_truth.is_some(), "truth is flowing");

    // ...and still no state-dependent command can be constructed.
    let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
}

#[test]
fn estimate_loss_rejects_control_while_truth_stays_healthy() {
    let (name, _writer) = truth_writer("los");
    let uplink = crate::uplink::FlightUplink::new().expect("uplink");
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        state_with(Duration::from_secs(10), Duration::from_secs(10)),
    )
    .with_uplink(uplink);
    attach_truth(&mut adapter, &name);

    let batch = adapter.sample_telemetry();
    let sample = &batch.samples[0];
    assert!(sample.avionics.is_none(), "stale estimate is withheld");
    assert!(sample.sim_truth.is_some(), "truth keeps flowing");

    let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
}

#[test]
fn fc_state_publishes_standalone_with_host_receive_provenance() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind fake FC");
    let mut uplink = crate::uplink::FlightUplink::new().expect("uplink");
    // Accept the GCS-encoded test heartbeat's source ids.
    uplink.set_expected_source(255, 190);
    let uplink_addr = uplink.local_addr().expect("uplink addr");
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        Arc::new(Mutex::new(LinkState::default())),
    )
    .with_uplink(uplink);
    adapter.estimate = None;

    fc.send_to(
        &pilotage_mavlink::codec::encode_gcs_heartbeat(0),
        uplink_addr,
    )
    .expect("send heartbeat");
    // The receive is a kernel handoff on loopback; poll until the
    // datagram lands rather than sleeping.
    let mut batch = TelemetryBatch::default();
    for _ in 0..200 {
        batch = adapter.sample_telemetry();
        if !batch.samples.is_empty() {
            break;
        }
        std::thread::yield_now();
    }

    // A healthy heartbeat alone is a publishable observation: no
    // estimate, no truth, and still a stamped FC-state sample.
    let sample = &batch.samples[0];
    assert!(sample.avionics.is_none());
    assert!(sample.sim_truth.is_none());
    let fc_state = sample.fc_state.expect("stamped FC state");
    assert_eq!(fc_state.arm_state, 1, "heartbeat reports disarmed");
    assert_eq!(fc_state.stamp.role, SourceRole::FcState);
    assert_eq!(fc_state.stamp.clock, MeasurementClock::HostMonotonic);
    assert_eq!(fc_state.stamp.sequence, 0);
    assert_eq!(
        fc_state.stamp.source_id,
        (255 << 8) | 190,
        "source id carries the configured FC (system, component) identity"
    );
    assert_eq!(
        fc_state.stamp.integrity,
        pilotage_adapter_api::SourceIntegrity::ChecksummedOnly,
        "CRC-only MAVLink is never labeled authenticated"
    );
}

#[test]
fn oracle_only_shape_advertises_no_control_and_streams_truth() {
    let (name, _writer) = truth_writer("orc");
    let mut adapter = AviateAdapter::from_state(
        VehicleId::new(1),
        Arc::new(Mutex::new(LinkState::default())),
    );
    adapter.estimate = None;
    attach_truth(&mut adapter, &name);

    // No uplink: motion control is structurally absent, not rejected
    // case by case.
    let capabilities = adapter.capabilities();
    assert!(capabilities.vehicles[0].scopes.is_empty());
    assert!(capabilities.vehicles[0].link_loss_actions.is_empty());
    let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::UnknownScope)
    );

    // The oracle remains independently observable for comparison
    // consumers, with no estimate synthesized alongside it.
    let batch = adapter.sample_telemetry();
    let sample = &batch.samples[0];
    assert!(sample.sim_truth.is_some());
    assert!(sample.avionics.is_none());
}

#[test]
fn mislabeled_estimate_roles_cannot_seed_a_control_setpoint() {
    let uplink = crate::uplink::FlightUplink::new().expect("uplink");
    let state = state_with(Duration::ZERO, Duration::ZERO);
    // A cache whose stamps claim the truth role is not an operational
    // estimate, whatever transport delivered it: the control boundary
    // gates on the exact role.
    {
        let mut latest = state.lock().expect("lock");
        if let Some(attitude) = latest.attitude.as_mut() {
            attitude.stamp.role = SourceRole::SimulationTruth;
        }
        if let Some(kinematics) = latest.kinematics.as_mut() {
            kinematics.stamp.role = SourceRole::SimulationTruth;
        }
    }
    let mut adapter = AviateAdapter::from_state(VehicleId::new(1), state).with_uplink(uplink);

    let outcome = adapter.apply_control(&flight_frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::MeasurementUnavailable)
    );
}

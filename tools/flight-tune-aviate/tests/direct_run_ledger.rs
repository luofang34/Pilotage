//! The durable ledger that brackets every exact direct command.
//!
//! The transport can send a step. What these tests are about is what the
//! run knows afterwards: that the prepared intent was durable before a
//! datagram could leave, that the result is durable after, that a record
//! only becomes evidence once it agrees with the intent it closes, and
//! that a prepared intent with no result is reported as ambiguous rather
//! than guessed at.
//!
//! SIM / NOT FOR FLIGHT.

#![allow(clippy::expect_used, clippy::panic)]

#[path = "direct_run_ledger/sender.rs"]
mod sender;

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use flight_tune::{
    ArtifactIdentity, ControlChannel, ControlFamily, Digest, ExecutionTarget, PhysicalUnit,
    ReferenceRule, SimulatorCapability, SimulatorSessionReceipt, StimulusEnvelope, StimulusMapping,
    VehicleBindingReceipt,
};
use flight_tune_aviate::direct_transport::{
    CausalReadbackBound, DirectBaselineRequest, DirectTransportRequest, direct_transport_identity,
};
use flight_tune_aviate::runtime::direct::{
    DirectBaselinePolicy, DirectControl, DirectEntryState, DirectRunAuthority, NoDirectControl,
    SimulatorDirectControl,
};
use flight_tune_aviate::runtime::phase::direct::DirectStepOutcome;
use flight_tune_aviate::runtime::phase::direct::ledger::{
    DirectIntentStore, DirectRecoveryOutcome, DirectSendOutcome, DurableDirectIntentStore,
};
use flight_tune_aviate::runtime::phase::direct::step_request;

use sender::{ENDPOINT, RecordingSender, SAMPLE_PERIOD_NS};

const TOLERANCE: f64 = 1e-9;
const HOVER_TRIM: f64 = 0.72;

/// One private ledger root that the test removes when it finishes.
struct LedgerRoot(PathBuf);

impl LedgerRoot {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("pilotage-469-ledger-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).expect("create the ledger root");
        // The store requires its root to be the canonical path, and the
        // system temporary directory reaches it through a symlink.
        let root = std::fs::canonicalize(&root).expect("canonicalize the ledger root");
        // The store keeps its root private to one user.
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("make the ledger root private");
        Self(root)
    }

    fn store(&self) -> DurableDirectIntentStore {
        DurableDirectIntentStore::open_blocking(&self.0).expect("open the direct ledger")
    }
}

impl Drop for LedgerRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn digest(seed: u8) -> Digest {
    Digest::from_bytes([seed; 32])
}

fn run_intent() -> Digest {
    digest(9)
}

fn capability() -> SimulatorCapability {
    SimulatorCapability::for_test(session_receipt())
}

fn session_receipt() -> SimulatorSessionReceipt {
    SimulatorSessionReceipt {
        session_digest: digest(1),
        simulator_digest: digest(2),
        airframe_digest: digest(3),
    }
}

fn vehicle_receipt() -> VehicleBindingReceipt {
    VehicleBindingReceipt {
        session_digest: digest(1),
        vehicle_digest: digest(4),
        scenario_runtime_digest: digest(5),
    }
}

fn runtime_identity() -> ArtifactIdentity {
    ArtifactIdentity::new("pilotage-aviate-test-runtime", digest(6))
        .expect("a named runtime identity")
}

fn transport_identity() -> ArtifactIdentity {
    direct_transport_identity().expect("the direct transport identity")
}

fn envelope() -> StimulusEnvelope {
    StimulusEnvelope {
        id: "alia-tilt-v1".to_owned(),
        revision: 1,
        unit: PhysicalUnit::Radians,
        reference: ReferenceRule::EffectiveSetpointAtEntry,
        negative_endpoint: -0.25,
        neutral: 0.0,
        positive_endpoint: 0.25,
    }
}

fn baseline_request() -> DirectBaselineRequest {
    DirectBaselineRequest {
        measured_roll_rad: 0.01,
        measured_pitch_rad: -0.02,
        measured_yaw_rad: 1.2,
        hover_trim: HOVER_TRIM,
        run_intent_digest: run_intent(),
        max_commands: 8,
    }
}

fn control(
    sender: RecordingSender,
    store: DurableDirectIntentStore,
) -> SimulatorDirectControl<RecordingSender, DurableDirectIntentStore> {
    let capability = capability();
    let simulator = session_receipt();
    let vehicle = vehicle_receipt();
    let transport = transport_identity();
    let readback =
        CausalReadbackBound::new(SAMPLE_PERIOD_NS, SAMPLE_PERIOD_NS).expect("a readback bound");
    SimulatorDirectControl::authorize(
        &DirectTransportRequest {
            capability: &capability,
            simulator: &simulator,
            vehicle: &vehicle,
            target: ExecutionTarget::Simulator,
            transport: &transport,
            readback,
            tolerance: TOLERANCE,
        },
        sender,
        store,
        run_intent(),
        baseline_policy(),
    )
    .expect("authorize the simulator-only direct path")
}

/// The configured shape of the test run's baseline block.
fn baseline_policy() -> DirectBaselinePolicy {
    DirectBaselinePolicy {
        hover_trim: HOVER_TRIM,
        max_commands: 8,
    }
}

/// The vehicle state the test enters its direct stimulus from.
const fn entry() -> DirectEntryState {
    DirectEntryState {
        roll_rad: 0.01,
        pitch_rad: -0.02,
        yaw_rad: 1.2,
    }
}

fn step(normalized: f64) -> flight_tune_aviate::direct_transport::DirectStepRequest {
    step_request(
        ControlFamily::DirectAttitudeThrust,
        ControlChannel::Pitch,
        StimulusMapping::AffineExact,
        &envelope(),
        normalized,
    )
    .expect("a direct step request")
}

#[test]
fn an_enacted_step_leaves_a_resolved_ledger_and_becomes_evidence() {
    let root = LedgerRoot::new("enacted");
    let mut direct = control(RecordingSender::new(), root.store());
    direct
        .ensure_baseline_blocking(entry())
        .expect("freeze the direct baseline");

    assert_eq!(
        direct
            .command_blocking(&step(0.5), false)
            .expect("send the exact step"),
        DirectStepOutcome::Enacted
    );

    let evidence = direct
        .seal(&runtime_identity())
        .expect("seal the direct evidence")
        .expect("the run commanded directly");
    assert_eq!(evidence.records.len(), 1);
    assert_eq!(evidence.run_intent_digest, run_intent());
    evidence
        .require_bound(run_intent(), &runtime_identity())
        .expect("the evidence binds its run and runtime");
    evidence
        .require_bound(digest(0x77), &runtime_identity())
        .expect_err("evidence for another run intent must fail closed");

    // The ledger says the same thing the evidence does. The run has to
    // release its writer lease before another reader can take it.
    drop(direct);
    let store = root.store();
    match store.read_state(0).expect("read the ledger") {
        DirectRecoveryOutcome::Resolved(result) => {
            assert_eq!(result.run_intent_digest, run_intent());
            assert!(matches!(result.outcome, DirectSendOutcome::Enacted { .. }));
        }
        other => panic!("the ledger must resolve an enacted command: {other:?}"),
    }
}

#[test]
fn a_prepared_intent_is_durable_before_the_command_can_be_sent() {
    let root = LedgerRoot::new("durable-before-send");
    let mut direct = control(RecordingSender::new().silent_after_baseline(), root.store());
    direct
        .ensure_baseline_blocking(entry())
        .expect("freeze the direct baseline");

    // The raw source goes silent, so the transport sends nothing. The
    // prepared intent is on disk anyway, because it was written first.
    assert_eq!(
        direct
            .command_blocking(&step(0.5), false)
            .expect("attempt the exact step"),
        DirectStepOutcome::NoExactSource
    );
    // Nothing that sent nothing becomes evidence.
    let evidence = direct
        .seal(&runtime_identity())
        .expect("seal the direct evidence")
        .expect("the run held direct authority");
    drop(direct);
    let store = root.store();
    match store.read_state(0).expect("read the ledger") {
        DirectRecoveryOutcome::Resolved(result) => {
            assert!(matches!(
                result.outcome,
                DirectSendOutcome::NoExactSource {}
            ));
        }
        other => panic!("a command that sent nothing must still resolve: {other:?}"),
    }
    assert!(evidence.records.is_empty());
}

#[test]
fn a_prepared_intent_with_no_durable_result_is_ambiguous() {
    let root = LedgerRoot::new("ambiguous");
    {
        // A run that stops between the durable intent and the durable
        // result leaves exactly this state on disk.
        let mut store = root.store();
        let mut authority = authority(RecordingSender::new());
        let prepared = prepare_one(&mut authority);
        authority
            .ledger_mut()
            .prepare(&mut store, &prepared)
            .expect("make the prepared intent durable");
    }

    {
        let store = root.store();
        match store.read_state(0).expect("read the ledger") {
            DirectRecoveryOutcome::Ambiguous(intent) => {
                assert_eq!(intent.sequence, 0);
                assert_eq!(intent.run_intent_digest, run_intent());
            }
            other => panic!("an unresolved prepared intent must be ambiguous: {other:?}"),
        }
    }

    // A run cannot resume through it: authorizing the direct path reads
    // the ledger first and refuses.
    let capability = capability();
    let simulator = session_receipt();
    let vehicle = vehicle_receipt();
    let transport = transport_identity();
    let readback =
        CausalReadbackBound::new(SAMPLE_PERIOD_NS, SAMPLE_PERIOD_NS).expect("a readback bound");
    let resume = SimulatorDirectControl::authorize(
        &DirectTransportRequest {
            capability: &capability,
            simulator: &simulator,
            vehicle: &vehicle,
            target: ExecutionTarget::Simulator,
            transport: &transport,
            readback,
            tolerance: TOLERANCE,
        },
        RecordingSender::new(),
        root.store(),
        run_intent(),
        baseline_policy(),
    );
    match resume {
        Ok(_) => panic!("an ambiguous ledger must refuse a resume"),
        Err(error) => {
            let detail = error.to_string();
            assert!(detail.contains("ambiguous"), "{detail}");
        }
    }
}

#[test]
fn a_record_that_leaves_the_declared_tolerance_never_becomes_evidence() {
    let root = LedgerRoot::new("substituted");
    // The flight controller reports a setpoint that is not the transmitted
    // one, so the transport itself refuses before publication.
    let mut direct = control(RecordingSender::new().substituting_pitch(0.9), root.store());
    direct
        .ensure_baseline_blocking(entry())
        .expect_err("a baseline the controller does not take must not freeze");
}

#[test]
fn a_runtime_with_no_direct_authority_refuses_the_direct_family() {
    let mut direct = NoDirectControl;
    let detail = direct
        .command_blocking(&step(0.5), false)
        .expect_err("a runtime with no direct authority must refuse")
        .to_string();
    assert!(detail.contains("no direct authority"), "{detail}");
    direct
        .ensure_baseline_blocking(entry())
        .expect_err("a runtime with no direct authority cannot freeze a baseline");
    assert!(
        direct
            .seal(&runtime_identity())
            .expect("a run with no direct path seals no direct evidence")
            .is_none()
    );
}

#[test]
fn the_operator_velocity_family_never_reaches_the_direct_path() {
    let detail = step_request(
        ControlFamily::OperatorVelocity,
        ControlChannel::Pitch,
        StimulusMapping::CandidateBoundCurve,
        &envelope(),
        0.5,
    )
    .expect_err("the operator velocity family must not reach the direct path")
    .to_string();
    assert!(detail.contains("operator_velocity"), "{detail}");
}

#[test]
fn an_inexact_mapping_never_reaches_the_direct_path() {
    let detail = step_request(
        ControlFamily::DirectAttitudeThrust,
        ControlChannel::Pitch,
        StimulusMapping::CandidateBoundCurve,
        &envelope(),
        0.5,
    )
    .expect_err("an inexact mapping must not reach the direct path")
    .to_string();
    assert!(detail.contains("candidate_bound_curve"), "{detail}");
}

#[test]
fn the_ledger_endpoint_is_the_normal_command_stream() {
    let root = LedgerRoot::new("endpoint");
    let mut direct = control(RecordingSender::new(), root.store());
    direct
        .ensure_baseline_blocking(entry())
        .expect("freeze the direct baseline");
    direct
        .command_blocking(&step(0.25), false)
        .expect("send the exact step");
    let evidence = direct
        .seal(&runtime_identity())
        .expect("seal the direct evidence")
        .expect("the run commanded directly");
    assert_eq!(evidence.records[0].sender.endpoint, ENDPOINT);
}

/// One direct authority over a sender, with no durable store attached.
fn authority(sender: RecordingSender) -> DirectRunAuthority {
    let capability = capability();
    let simulator = session_receipt();
    let vehicle = vehicle_receipt();
    let transport = transport_identity();
    let readback =
        CausalReadbackBound::new(SAMPLE_PERIOD_NS, SAMPLE_PERIOD_NS).expect("a readback bound");
    DirectRunAuthority::authorize(
        &DirectTransportRequest {
            capability: &capability,
            simulator: &simulator,
            vehicle: &vehicle,
            target: ExecutionTarget::Simulator,
            transport: &transport,
            readback,
            tolerance: TOLERANCE,
        },
        &sender,
        run_intent(),
    )
    .expect("authorize the direct path")
}

/// One prepared direct command over a frozen baseline.
fn prepare_one(
    authority: &mut DirectRunAuthority,
) -> flight_tune_aviate::direct_transport::PreparedDirectCommand {
    let mut sender = RecordingSender::new();
    authority
        .transport_mut()
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("freeze the direct baseline");
    authority
        .transport()
        .prepare_step(&step(0.5))
        .expect("prepare the exact step")
}

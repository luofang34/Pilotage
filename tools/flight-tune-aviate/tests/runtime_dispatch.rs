//! One production Aviate run, driven directive by directive.
//!
//! Everything else in this suite checks a part. This drives the whole
//! vehicle port the way a campaign does: admit a run, start it, feed it
//! ordered frames with the directives a trial issues, and read the
//! receipts and the seal that come back.
//!
//! SIM / NOT FOR FLIGHT.

#![allow(clippy::expect_used, clippy::panic)]

// Both helpers are shared with the suites that own them, which ask them
// for behaviors this one does not need.
#[path = "production_binding/rig.rs"]
#[allow(dead_code)]
mod rig;
#[path = "direct_run_ledger/sender.rs"]
#[allow(dead_code)]
mod sender;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use flight_tune::{
    CanonicalTelemetryKey, Digest, ExecutionTarget, KinematicTruth, MissionCapability,
    MissionDirective, MissionTerminal, ReceiptResult, ScenarioFrame, ScenarioStopContext,
    ScenarioStopReason, SimulatorSessionReceipt, VehicleBindingReceipt, VehicleLifecycleState,
};
use flight_tune::{ScenarioRuntime, scenario_runtime_identity};
use flight_tune_aviate::direct_transport::{
    CausalReadbackBound, DirectTransportRequest, direct_transport_identity,
};
use flight_tune_aviate::runtime::direct::{
    DirectBaselinePolicy, NoDirectControl, SimulatorDirectControl,
};
use flight_tune_aviate::runtime::phase::direct::ledger::DurableDirectIntentStore;
use flight_tune_aviate::runtime::phase::transition::StartStateTolerance;
use flight_tune_aviate::{
    AviateScenarioDriver, AviateVehicleActionPort, aviate_action_port_identity,
};

use rig::{candidate, candidate_digest, mission_document, run_context, runtime_identity};
use sender::{RecordingSender, SAMPLE_PERIOD_NS};

/// The vertical envelope a direct collective stimulus resolves through.
const VERTICAL_ENVELOPE: &str = r#"{
    "id": "alia250.direct.collective",
    "revision": 2,
    "unit": "normalized_collective_force",
    "reference": "identified_hover_trim",
    "negative_endpoint": -0.2,
    "neutral": 0.05,
    "positive_endpoint": 0.4
}"#;

fn directive(action_id: u32, phase_id: &str, action: serde_json::Value) -> MissionDirective {
    serde_json::from_value(serde_json::json!({
        "lane": "trial",
        "directive": {
            "context": {
                "action_id": action_id,
                "phase_index": 0,
                "phase_id": phase_id,
                "attempt": 1,
                "purpose": { "purpose": "phase_action" }
            },
            "action": action
        }
    }))
    .expect("a typed trial directive")
}

fn wait_ready() -> MissionDirective {
    directive(1, "ready", serde_json::json!({ "kind": "wait_ready" }))
}

fn observe() -> MissionDirective {
    directive(2, "observe", serde_json::json!({ "kind": "observe" }))
}

fn stimulate(value: f64) -> MissionDirective {
    let envelope: serde_json::Value =
        serde_json::from_str(VERTICAL_ENVELOPE).expect("a valid envelope");
    directive(
        3,
        "stimulus",
        serde_json::json!({
            "kind": "stimulate",
            "family": "direct_attitude_thrust",
            "channel": "vertical",
            "mapping": "affine_exact",
            "envelope": envelope,
            "waveform": { "kind": "step", "value": value }
        }),
    )
}

fn release() -> MissionDirective {
    directive(
        4,
        "release",
        serde_json::json!({ "kind": "release_control" }),
    )
}

fn disarm() -> MissionDirective {
    directive(5, "disarm", serde_json::json!({ "kind": "disarm" }))
}

fn frame(source_sequence: u64, lifecycle: Option<VehicleLifecycleState>) -> ScenarioFrame {
    ScenarioFrame {
        source_sequence,
        simulator_time_ns: source_sequence * SAMPLE_PERIOD_NS,
        trial_time_ns: source_sequence * SAMPLE_PERIOD_NS,
        lifecycle,
        ground_contact: Some(false),
        crashed: Some(false),
        link_valid: Some(true),
        estimator_valid: Some(true),
        truth: KinematicTruth {
            position_ned_m: [0.0; 3],
            velocity_ned_mps: [0.0; 3],
            acceleration_ned_mps2: [0.0; 3],
            attitude_wxyz: [1.0, 0.0, 0.0, 0.0],
            body_rates_rps: [0.0; 3],
        },
        applied_conditions: BTreeMap::new(),
        canonical_signals: Vec::new(),
    }
}

/// One private ledger root that the test removes when it finishes.
struct LedgerRoot(PathBuf);

impl LedgerRoot {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "pilotage-469-dispatch-{}-{name}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).expect("create the ledger root");
        let root = std::fs::canonicalize(&root).expect("canonicalize the ledger root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("make the ledger root private");
        Self(root)
    }
}

impl Drop for LedgerRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn direct_control(root: &LedgerRoot, run_intent_digest: Digest) -> Direct {
    let capability = rig::capability(0x11);
    let simulator = SimulatorSessionReceipt {
        session_digest: Digest::from_bytes([0x11; 32]),
        simulator_digest: Digest::from_bytes([0x51; 32]),
        airframe_digest: Digest::from_bytes([0x52; 32]),
    };
    let vehicle = VehicleBindingReceipt {
        session_digest: Digest::from_bytes([0x11; 32]),
        vehicle_digest: Digest::from_bytes([0x22; 32]),
        scenario_runtime_digest: Digest::from_bytes([0x23; 32]),
    };
    let transport = direct_transport_identity().expect("the direct transport identity");
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
            tolerance: 1e-9,
        },
        RecordingSender::new(),
        DurableDirectIntentStore::open_blocking(&root.0).expect("open the direct ledger"),
        run_intent_digest,
        DirectBaselinePolicy {
            hover_trim: 0.72,
            max_commands: 8,
        },
    )
    .expect("authorize the simulator-only direct path")
}

fn tolerance() -> StartStateTolerance {
    StartStateTolerance {
        position_m: 0.5,
        heading_rad: 0.1,
        speed_mps: 0.2,
        dwell_ns: 25_000_000,
    }
}

fn capabilities() -> Vec<MissionCapability> {
    vec![
        MissionCapability::KinematicTruth,
        MissionCapability::ArmDisarm,
        MissionCapability::DirectAttitudeThrustControl,
    ]
}

#[test]
fn a_run_advances_every_directive_and_seals_its_direct_evidence() {
    let candidate = candidate(0.06, 0.35, 4.0);
    let document = mission_document("dispatch");
    let context = run_context(0x11, &document, candidate_digest(&candidate), 61);
    let intent = context.digest().expect("the run intent digest");
    let root = LedgerRoot::new("enacted");
    let runtime = runtime_identity("dispatch");
    let frozen = runtime.identity().clone();
    let driver = AviateScenarioDriver::new(
        runtime,
        capabilities(),
        tolerance(),
        direct_control(&root, intent),
    )
    .expect("a driver with a direct path");

    let mut port = AviateVehicleActionPort::new(driver).expect("the composed action port");
    assert_eq!(
        port.identity(),
        &scenario_runtime_identity(
            &aviate_action_port_identity(&frozen).expect("the action port identity")
        )
        .expect("the composed scenario runtime identity")
    );

    port.prepare_blocking(&document, &context)
        .expect("admit the run");
    port.start_blocking().expect("start the run");

    // A frame with no directive advances nothing and returns no receipt.
    assert_eq!(
        port.observe_blocking(&frame(1, Some(VehicleLifecycleState::Armed)), None)
            .expect("observe an idle frame")
            .action_result,
        None
    );

    advance_the_trial(&mut port);

    let mut stop = ScenarioStopContext {
        reason: ScenarioStopReason::Mission(MissionTerminal::Complete {
            completed_phases: 5,
        }),
        last_source_sequence: None,
    };
    port.stop_blocking(&mut stop).expect("stop the run");
    let sealed = port.driver().seal().expect("a sealed run").clone();
    assert_eq!(sealed.run_intent_digest, intent);
    assert_eq!(sealed.runtime_identity, frozen);
    assert_eq!(sealed.accepted_frames, 6);
    assert_eq!(stop.last_source_sequence, Some(6));
    assert!(
        sealed.direct_evidence_digest.is_some(),
        "a run that commanded directly seals its direct evidence"
    );
    port.cleanup_blocking().expect("clean up the run");
}

#[test]
fn a_direct_stimulus_on_a_runtime_with_no_direct_path_is_refused() {
    let candidate = candidate(0.06, 0.35, 4.0);
    let document = mission_document("no-direct-path");
    let context = run_context(0x11, &document, candidate_digest(&candidate), 62);
    let driver = AviateScenarioDriver::new(
        runtime_identity("no-direct-path"),
        capabilities(),
        tolerance(),
        NoDirectControl,
    )
    .expect("a driver with no direct path");
    let mut port = AviateVehicleActionPort::new(driver).expect("the composed action port");
    port.prepare_blocking(&document, &context)
        .expect("admit the run");
    port.start_blocking().expect("start the run");
    port.observe_blocking(
        &frame(1, Some(VehicleLifecycleState::Armed)),
        Some(&wait_ready()),
    )
    .expect("advance the readiness wait");

    let detail = port
        .observe_blocking(
            &frame(2, Some(VehicleLifecycleState::Armed)),
            Some(&stimulate(0.5)),
        )
        .expect_err("a direct stimulus must be refused with no direct path")
        .to_string();
    assert!(detail.contains("no direct authority"), "{detail}");

    // A run with no direct path seals no direct evidence.
    let mut stop = ScenarioStopContext {
        reason: ScenarioStopReason::ExecutionError,
        last_source_sequence: None,
    };
    port.stop_blocking(&mut stop).expect("stop the run");
    assert_eq!(
        port.driver()
            .seal()
            .expect("a sealed run")
            .direct_evidence_digest,
        None
    );
}

#[test]
fn a_frame_out_of_order_stops_the_run_before_the_directive_advances() {
    let candidate = candidate(0.06, 0.35, 4.0);
    let document = mission_document("frame-order");
    let context = run_context(0x11, &document, candidate_digest(&candidate), 63);
    let driver = AviateScenarioDriver::new(
        runtime_identity("frame-order"),
        capabilities(),
        tolerance(),
        NoDirectControl,
    )
    .expect("a driver");
    let mut port = AviateVehicleActionPort::new(driver).expect("the composed action port");
    port.prepare_blocking(&document, &context)
        .expect("admit the run");
    port.start_blocking().expect("start the run");
    port.observe_blocking(
        &frame(4, Some(VehicleLifecycleState::Armed)),
        Some(&observe()),
    )
    .expect("advance the first frame");

    let detail = port
        .observe_blocking(
            &frame(4, Some(VehicleLifecycleState::Armed)),
            Some(&observe()),
        )
        .expect_err("a repeated frame must stop the run")
        .to_string();
    assert!(detail.contains("does not advance"), "{detail}");
}

#[test]
fn a_run_cannot_observe_before_it_starts() {
    let candidate = candidate(0.06, 0.35, 4.0);
    let document = mission_document("unstarted");
    let context = run_context(0x11, &document, candidate_digest(&candidate), 64);
    let driver = AviateScenarioDriver::new(
        runtime_identity("unstarted"),
        capabilities(),
        tolerance(),
        NoDirectControl,
    )
    .expect("a driver");
    let mut port = AviateVehicleActionPort::new(driver).expect("the composed action port");
    port.observe_blocking(&frame(1, None), Some(&observe()))
        .expect_err("an unstarted run cannot observe");
    port.prepare_blocking(&document, &context)
        .expect("admit the run");
    port.observe_blocking(&frame(1, None), Some(&observe()))
        .expect_err("an admitted run that has not started cannot observe");
    port.start_blocking().expect("start the run");
    port.observe_blocking(&frame(1, None), Some(&observe()))
        .expect("a started run observes");
}

/// The direct path this suite drives the vehicle port through.
type Direct = SimulatorDirectControl<RecordingSender, DurableDirectIntentStore>;

/// Advances the trial's directives and checks each receipt as it lands.
fn advance_the_trial(port: &mut AviateVehicleActionPort<AviateScenarioDriver<Direct>>) {
    for (sequence, directive) in [
        (2, wait_ready()),
        (3, stimulate(0.5)),
        (4, release()),
        (5, observe()),
        (6, disarm()),
    ] {
        let lifecycle = if sequence == 6 {
            VehicleLifecycleState::Disarmed
        } else {
            VehicleLifecycleState::Armed
        };
        let receipt = port
            .observe_blocking(&frame(sequence, Some(lifecycle)), Some(&directive))
            .unwrap_or_else(|error| panic!("advance directive {sequence}: {error}"));
        assert_eq!(receipt.source_sequence, sequence);
        assert_eq!(
            receipt.action_result,
            Some(ReceiptResult::Succeeded {}),
            "directive {sequence} must complete"
        );
        // The commanded value reaches the canonical telemetry on the frame
        // that commands it, and the release that follows clears it.
        let telemetry = port
            .driver()
            .canonical_telemetry()
            .expect("the canonical telemetry");
        let expected = if sequence == 3 { 0.5 } else { 0.0 };
        assert!(
            (telemetry[CanonicalTelemetryKey::CommandPrimary.as_str()] - expected).abs()
                < f64::EPSILON,
            "frame {sequence} commanded {}",
            telemetry[CanonicalTelemetryKey::CommandPrimary.as_str()]
        );
        assert!(telemetry[CanonicalTelemetryKey::CommandLinkValid.as_str()] > 0.5);
    }
}

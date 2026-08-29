#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use flight_tune::{Digest, KinematicTruth};

use super::*;

struct RecordingDriver {
    identity: ArtifactIdentity,
    directives: Vec<AviateVehicleDirective>,
}

impl AviateActionDriver for RecordingDriver {
    fn action_port_identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn capabilities(&self) -> &[MissionCapability] {
        &[
            MissionCapability::SimulatorTime,
            MissionCapability::ArmDisarm,
            MissionCapability::SimulatorControl,
        ]
    }

    fn prepare_blocking(
        &mut self,
        _document: &MissionDocument,
        _context: &RunExecutionContext,
    ) -> Result<(), AviateActionPortError> {
        Ok(())
    }

    fn start_blocking(&mut self) -> Result<(), AviateActionPortError> {
        Ok(())
    }

    fn observe_blocking(
        &mut self,
        _frame: &ScenarioFrame,
        directive: Option<&AviateVehicleDirective>,
    ) -> Result<Option<ReceiptResult>, AviateActionPortError> {
        if let Some(directive) = directive {
            self.directives.push(directive.clone());
            Ok(Some(ReceiptResult::Succeeded {}))
        } else {
            Ok(None)
        }
    }

    fn stop_blocking(
        &mut self,
        _context: &mut ScenarioStopContext,
    ) -> Result<(), AviateActionPortError> {
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), AviateActionPortError> {
        Ok(())
    }
}

#[test]
fn port_preserves_the_typed_directive_and_frame_sequence() {
    let driver_identity = ArtifactIdentity::new("aviate-driver", Digest::from_bytes([7; 32]))
        .expect("driver identity");
    let driver = RecordingDriver {
        identity: driver_identity.clone(),
        directives: Vec::new(),
    };
    let mut port = AviateVehicleActionPort::new(driver).expect("action port");
    let expected_action_port =
        aviate_action_port_identity(&driver_identity).expect("action-port identity");
    let expected_runtime =
        scenario_runtime_identity(&expected_action_port).expect("runtime identity");
    assert_eq!(port.identity(), &expected_runtime);
    let directive: MissionDirective = serde_json::from_value(serde_json::json!({
        "lane": "trial",
        "directive": {
            "context": {
                "action_id": 1,
                "phase_index": 0,
                "phase_id": "observe",
                "attempt": 1,
                "purpose": { "purpose": "phase_action" }
            },
            "action": { "kind": "observe" }
        }
    }))
    .expect("typed directive");
    let frame = frame(19);

    let receipt = ScenarioRuntime::observe_blocking(&mut port, &frame, Some(&directive))
        .expect("observe directive");

    assert_eq!(receipt.source_sequence, 19);
    assert_eq!(receipt.action_result, Some(ReceiptResult::Succeeded {}));
    let projected = port.into_inner().directives;
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].context, directive.context().clone());
    assert_eq!(projected[0].action, AviateVehicleAction::Observe);
}

#[test]
fn the_port_carries_the_typed_control_family_and_envelope_to_the_driver() {
    let driver_identity = ArtifactIdentity::new("aviate-driver", Digest::from_bytes([7; 32]))
        .expect("driver identity");
    let mut port = AviateVehicleActionPort::new(RecordingDriver {
        identity: driver_identity,
        directives: Vec::new(),
    })
    .expect("action port");
    let directive: MissionDirective = serde_json::from_value(serde_json::json!({
        "lane": "trial",
        "directive": {
            "context": {
                "action_id": 4,
                "phase_index": 0,
                "phase_id": "stimulus",
                "attempt": 1,
                "purpose": { "purpose": "phase_action" }
            },
            "action": {
                "kind": "stimulate",
                "family": "direct_attitude_thrust",
                "channel": "vertical",
                "mapping": "affine_exact",
                "envelope": {
                    "id": "alia250.direct.collective",
                    "revision": 2,
                    "unit": "normalized_collective_force",
                    "reference": "identified_hover_trim",
                    "negative_endpoint": -0.2,
                    "neutral": 0.05,
                    "positive_endpoint": 0.4
                },
                "waveform": { "kind": "step", "value": 0.5 }
            }
        }
    }))
    .expect("typed stimulus directive");

    let receipt = ScenarioRuntime::observe_blocking(&mut port, &frame(21), Some(&directive))
        .expect("observe directive");

    assert_eq!(receipt.action_result, Some(ReceiptResult::Succeeded {}));
    let projected = port.into_inner().directives;
    let AviateVehicleAction::Stimulate {
        family,
        channel,
        mapping,
        envelope,
        ..
    } = &projected[0].action
    else {
        panic!("the port must carry the stimulus to the driver");
    };
    assert_eq!(*family, ControlFamily::DirectAttitudeThrust);
    assert_eq!(*channel, ControlChannel::Vertical);
    assert_eq!(*mapping, StimulusMapping::AffineExact);
    assert!(
        (mapping
            .resolve_exact(envelope, 1.0)
            .expect("resolve the positive endpoint")
            - 0.4)
            .abs()
            < 1.0e-12
    );
}

#[test]
fn simulator_actions_do_not_reach_the_vehicle_driver() {
    let driver_identity = ArtifactIdentity::new("aviate-driver", Digest::from_bytes([7; 32]))
        .expect("driver identity");
    let mut port = AviateVehicleActionPort::new(RecordingDriver {
        identity: driver_identity,
        directives: Vec::new(),
    })
    .expect("action port");
    let directive: MissionDirective = serde_json::from_value(serde_json::json!({
        "lane": "trial",
        "directive": {
            "context": {
                "action_id": 1,
                "phase_index": 0,
                "phase_id": "reset",
                "attempt": 1,
                "purpose": { "purpose": "phase_action" }
            },
            "action": { "kind": "reset" }
        }
    }))
    .expect("typed directive");

    let receipt = port
        .observe_blocking(&frame(3), Some(&directive))
        .expect("observe directive");

    assert!(matches!(
        receipt.action_result,
        Some(ReceiptResult::Refused { .. })
    ));
    assert!(port.into_inner().directives.is_empty());
}

fn frame(source_sequence: u64) -> ScenarioFrame {
    ScenarioFrame {
        source_sequence,
        simulator_time_ns: 10,
        trial_time_ns: 10,
        lifecycle: None,
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

/// One sensor-noise condition, in the form a campaign artifact carries it.
const SENSOR_NOISE_CONDITION: &str = r#"{
    "schema_version": 4,
    "id": "sensor-noise",
    "revision": 1,
    "seed": 5,
    "wind": {
        "steady": {"speed_mps": 0.0, "direction_deg": 0.0},
        "gusts": [],
        "turbulence": {"kind": "none"}
    },
    "timing": {"estimate_delay_ns": 0, "update_jitter": {"kind": "none"}},
    "sensor": {
        "kind": "bounded_noise",
        "lanes": [{
            "sensor": "gyroscope",
            "axis": "x",
            "peak_amplitude_rad_s": 0.01,
            "update_interval_samples": 5
        }]
    },
    "actuator": {
        "authority_scale_basis_points": 10000,
        "command_loss": {"kind": "none"}
    },
    "controller_initialization": {
        "hover_thrust_force": {"kind": "scale_baseline", "scale_basis_points": 10000}
    },
    "plant": {
        "payload_mass_delta_kg": 0.0,
        "longitudinal_cg_offset_m": 0.0,
        "lateral_cg_offset_m": 0.0,
        "hover_thrust_expectation": {"kind": "measured_weight_ratio"}
    }
}"#;

#[test]
fn the_action_port_declares_no_uncertainty_until_the_runtime_proves_one() {
    let driver_identity = ArtifactIdentity::new("aviate-driver", Digest::from_bytes([7; 32]))
        .expect("driver identity");
    let port = AviateVehicleActionPort::new(RecordingDriver {
        identity: driver_identity,
        directives: Vec::new(),
    })
    .expect("action port");
    let condition = flight_tune::ConditionSet::from_json(SENSOR_NOISE_CONDITION.as_bytes())
        .expect("sensor-noise condition");
    let admission = flight_tune::ConditionAdmission::new(
        flight_tune::UncertaintyDeclaration::from_runtime(&port),
    );

    assert!(port.uncertainty_capabilities().is_empty());
    assert_eq!(
        port.hover_estimator_mode(),
        flight_tune::HoverEstimatorMode::Online
    );
    assert!(matches!(
        admission.prepare(&condition),
        Err(ScenarioRuntimeError::UnsupportedCondition { .. })
    ));
}

#![allow(clippy::expect_used)]

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

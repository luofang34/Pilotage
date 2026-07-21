//! The host half of the typed-control E2E chain (CTRL-01): decodes the SAME
//! wire bytes the browser half produced (keyboard through the real wasm
//! runtime, scaled by the advertised envelope, encoded by wire.js — see
//! `clients/web/typed-command.test.mjs` and the shared fixture
//! `clients/web-control/typed-frame-fixture.json`), then drives them through
//! the session engine's welcome/lease/gate path and asserts the exact typed
//! command that reaches the adapter boundary. One fixture, both ends: the
//! chain cannot drift.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, ScopeDescriptor,
    VehicleDescriptor,
};
use pilotage_protocol::{
    ClientHello, ControlAction, ControlIntent, IntentFamily, LeaseRequest, LogicalAxisId,
    ModeTarget, ReferenceFrame, ScopeId, ScopedControlFrame, VehicleId,
    decode_control_frame_envelope,
};
use pilotage_session::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

const FIXTURE: &str = include_str!("../../../clients/web-control/typed-frame-fixture.json");

fn fixture_frame_bytes() -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    let hex = fixture["envelopeHex"].as_str().expect("hex present");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

/// A capability report matching the fixture's advertised envelope (the
/// px4-class 3.0/1.5/0.9 velocity family with arm + fpv-direct mode).
fn capabilities() -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VehicleId::new(1),
            scopes: vec![ScopeDescriptor {
                scope: ScopeId::new("vehicle.motion"),
                axes: vec![LogicalAxisId::new(0)],
                intents: vec![IntentCapability {
                    family: IntentFamily::Velocity,
                    frames: vec![ReferenceFrame::BodyFrd],
                    max_linear: 3.0,
                    max_vertical: 1.5,
                    max_angular: 0.9,
                }],
                actions: vec![
                    ActionCapability {
                        action: pilotage_protocol::ActionKind::Arm,
                        mode_targets: vec![],
                    },
                    ActionCapability {
                        action: pilotage_protocol::ActionKind::ModeRequest,
                        mode_targets: vec![ModeTarget::FpvDirect],
                    },
                ],
                legacy: None,
            }],
            link_loss_actions: vec![],
        }],
        adapter_version: "typed-e2e".to_owned(),
    }
}

/// Welcomes a client and grants it the fixture's scope, returning the
/// granted generation.
fn welcome_and_grant(
    engine: &mut SessionEngine,
    client: ClientKey,
    frame: &ScopedControlFrame,
) -> pilotage_protocol::Generation {
    let welcomed = engine.handle_client_message(
        client,
        DomainEnvelope::Hello(ClientHello {
            protocol_version: 1,
            client_name: "typed-e2e".to_owned(),
            join_token: vec![],
        }),
        MonoTimestamp::from_nanos(0),
    );
    assert!(!welcomed.actions.is_empty());
    let lease = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: frame.vehicle,
            scope: frame.scope.clone(),
        }),
        MonoTimestamp::from_nanos(1),
    );
    lease
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            } if response.granted => Some(response.generation),
            _ => None,
        })
        .expect("lease granted")
}

#[test]
fn browser_encoded_typed_frame_reaches_the_adapter_boundary_intact() {
    // 1. The browser-produced bytes decode as a typed-ONLY frame.
    let bytes = fixture_frame_bytes();
    let frame: ScopedControlFrame =
        decode_control_frame_envelope(&bytes).expect("typed-only frame decodes");
    assert!(!frame.carries_payload(), "no numeric payload on the wire");
    let Some(ControlIntent::Velocity(velocity)) = frame.intent else {
        panic!("expected the keyboard-climb velocity intent");
    };
    assert_eq!(velocity.frame, ReferenceFrame::BodyFrd);
    assert_eq!(velocity.vz, -1.5, "full climb at the advertised envelope");
    assert_eq!(velocity.vx, 0.0);
    assert_eq!(velocity.vy, 0.0);
    assert_eq!(velocity.yaw_rate, 0.0);
    assert_eq!(
        frame.actions,
        vec![
            ControlAction::Arm,
            ControlAction::ModeRequest {
                target: ModeTarget::FpvDirect
            },
        ],
        "typed actions with the explicit mode target"
    );

    // 2. The engine's welcome/lease/gate path forwards it to the adapter
    //    boundary unchanged.
    let mut engine = SessionEngine::new(
        capabilities(),
        StalenessPolicy::new(core::time::Duration::from_secs(1)),
        SessionConfig::new(1, "typed-e2e"),
    );
    let client = ClientKey::new(1);
    let generation = welcome_and_grant(&mut engine, client, &frame);

    let mut fenced = frame.clone();
    fenced.generation = generation;
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(fenced),
        // Matches the fixture's sampled_at, so the frame is fresh.
        MonoTimestamp::from_nanos(123_456_789),
    );
    let applied = outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::ApplyToAdapter { frame, .. } => Some(frame),
            _ => None,
        })
        .expect("the gate forwards the typed frame to the adapter");
    assert_eq!(
        applied.intent,
        Some(ControlIntent::Velocity(velocity)),
        "the adapter receives the exact typed velocity the browser encoded"
    );
    assert_eq!(applied.actions.len(), 2);
    assert!(!applied.carries_payload());
}

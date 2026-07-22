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

fn fixture_bytes(field: &str) -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    let hex = fixture[field].as_str().expect("hex present");
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
                    max_yaw_rate: 0.0,
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

/// Welcomes a client, announces the fixture's profile activation (typed
/// frames are rejected as `ProfileMismatch` without a matching
/// announcement), and grants it the fixture's scope, returning the granted
/// generation.
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
    let session = welcomed
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::Welcome(welcome),
                ..
            } => Some(welcome.session),
            _ => None,
        })
        .expect("welcomed");
    let announced = engine.handle_client_message(
        client,
        DomainEnvelope::ProfileActivation(pilotage_protocol::ProfileActivation {
            session,
            profile_id: "builtin.flight.default".to_owned(),
            profile_revision: 3,
            activation_revision: frame.activation_revision,
            digest: [0x11; 32],
            device_profile_id: String::new(),
            device_profile_revision: 0,
            device_digest: [0; 32],
        }),
        MonoTimestamp::from_nanos(0),
    );
    assert!(announced.actions.is_empty(), "announcement is recorded");
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
    // 1. The browser-produced bytes decode as a typed-ONLY setpoint frame:
    //    intent, no payload, and NO actions — those ride the reliable
    //    stream (CTRL-01).
    let bytes = fixture_bytes("envelopeHex");
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
    assert!(frame.actions.is_empty(), "setpoint frames carry no actions");

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
    assert!(!applied.carries_payload());
}

#[test]
fn browser_encoded_action_command_reaches_the_adapter_with_its_binding() {
    // The SAME bytes the browser test pinned: an Arm command on the
    // reliable stream, bound to session/vehicle/scope/generation/activation
    // revision with a nonzero correlation id.
    let bytes = fixture_bytes("actionCommandHex");
    let frame_bytes = fixture_bytes("envelopeHex");
    let frame: ScopedControlFrame =
        decode_control_frame_envelope(&frame_bytes).expect("frame decodes");

    let mut engine = SessionEngine::new(
        capabilities(),
        StalenessPolicy::new(core::time::Duration::from_secs(1)),
        SessionConfig::new(1, "typed-e2e"),
    );
    let client = ClientKey::new(1);
    let generation = welcome_and_grant(&mut engine, client, &frame);
    assert_eq!(generation.as_u64(), 1, "fixture command binds generation 1");

    let mut command = pilotage_protocol::decode_action_command_envelope(&bytes)
        .expect("action command envelope decodes");
    assert_eq!(command.action, ControlAction::Arm);
    assert_eq!(command.action_id, 1, "nonzero correlation id");
    // The fixture was minted against generation 4; rebind to the granted
    // generation so only the generation differs from the wire bytes.
    command.generation = generation;

    // But the browser session id (7) is not this engine's session: the
    // engine must CLOSE the connection for a forged session binding.
    let forged = engine.handle_client_message(
        client,
        DomainEnvelope::ActionCommand(command.clone()),
        MonoTimestamp::from_nanos(200_000_000),
    );
    assert!(
        forged
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::CloseClient { .. })),
        "a foreign session binding closes: {:?}",
        forged.actions
    );
}

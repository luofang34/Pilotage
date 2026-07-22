//! The capability-negotiation wire fixture (CTRL-01): the HOST'S OWN
//! encoder produces the ServerWelcome bytes committed in
//! `clients/web-control/server-welcome-fixture.json`, and the browser's
//! decoder is held to the SAME bytes (`clients/web/wire.test.mjs`). The
//! advertisement deliberately carries multi-value repeated enum lists
//! (frames, mode targets) so prost's PACKED encoding is what the browser
//! must parse — the encoding a naive per-entry varint decode turns to NaN.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    ActionCapability, AdapterCapabilities, ExecutionMode, IntentCapability, LegacyCommandMap,
    LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{
    ActionKind, ClientHello, IntentFamily, LogicalAxisId, ModeTarget, ReferenceFrame, ScopeId,
    VehicleId,
};
use pilotage_session::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
};
use pilotage_session_host::runtime::encode_envelope_message;
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

const FIXTURE: &str = include_str!("../../../clients/web-control/server-welcome-fixture.json");

/// A two-scope advertisement exercising every list shape the browser must
/// decode: multi-value reference frames, multi-value mode targets, the
/// attitude family's `max_yaw_rate`, and a legacy-translated scope.
fn capabilities() -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VehicleId::new(1),
            scopes: vec![
                ScopeDescriptor {
                    scope: ScopeId::new("vehicle.motion"),
                    axes: vec![LogicalAxisId::new(0), LogicalAxisId::new(1)],
                    intents: vec![IntentCapability {
                        family: IntentFamily::Velocity,
                        frames: vec![ReferenceFrame::BodyFrd, ReferenceFrame::LocalNed],
                        max_linear: 3.0,
                        max_vertical: 1.5,
                        max_angular: 0.9,
                        max_yaw_rate: 0.0,
                    }],
                    actions: vec![
                        ActionCapability {
                            action: ActionKind::Arm,
                            mode_targets: vec![],
                        },
                        ActionCapability {
                            action: ActionKind::ModeRequest,
                            mode_targets: vec![
                                ModeTarget::CameraVelocity,
                                ModeTarget::FpvDirect,
                                ModeTarget::Hold,
                            ],
                        },
                    ],
                    legacy: Some(LegacyCommandMap::Velocity {
                        vx: None,
                        vy: None,
                        vz: None,
                        yaw_rate: None,
                        arm_button: None,
                        disarm_button: None,
                        reset_button: None,
                    }),
                },
                ScopeDescriptor {
                    scope: ScopeId::new("vehicle.motion.direct"),
                    axes: vec![],
                    intents: vec![IntentCapability {
                        family: IntentFamily::AttitudeThrust,
                        frames: vec![ReferenceFrame::LocalNed],
                        max_linear: 0.0,
                        max_vertical: 0.0,
                        max_angular: 0.6,
                        max_yaw_rate: 0.9,
                    }],
                    actions: vec![ActionCapability {
                        action: ActionKind::Arm,
                        mode_targets: vec![],
                    }],
                    legacy: None,
                },
            ],
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
        }],
        adapter_version: "welcome-fixture".to_owned(),
    }
}

#[test]
fn the_committed_welcome_fixture_is_what_this_host_encodes() {
    let mut engine = SessionEngine::new(
        capabilities(),
        StalenessPolicy::new(core::time::Duration::from_secs(1)),
        SessionConfig::new(1, "welcome-fixture"),
    );
    let outcome = engine.handle_client_message(
        ClientKey::new(1),
        DomainEnvelope::Hello(ClientHello {
            protocol_version: 1,
            client_name: "fixture".to_owned(),
            join_token: vec![],
        }),
        MonoTimestamp::from_nanos(0),
    );
    let welcome = outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: envelope @ OutboundMessage::Welcome(_),
                ..
            } => Some(envelope),
            _ => None,
        })
        .expect("a welcome");
    let bytes = encode_envelope_message(welcome);
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();

    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture parses");
    let committed = fixture["envelopeHex"].as_str().expect("hex present");
    assert_eq!(
        hex, committed,
        "the committed ServerWelcome fixture no longer matches this host's \
         encoder; regenerate by replacing envelopeHex with the left value"
    );
}

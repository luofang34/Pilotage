//! Unit tests for the session engine, exercising the ADR-0005/0006/0009/0010
//! decision paths through the public API only.
//!
//! Each submodule covers one required behavior: handshake, lease grant and
//! duplicate request, accept-then-fence, staleness rejection, ping/pong,
//! disconnect release, and deadline pass-through.

#![allow(clippy::expect_used, clippy::panic)]

mod deadline;
mod disconnect;
mod frame;
mod handshake;
mod lease;
mod ping;
mod recovery;
mod release;
mod watchdog;

use core::time::Duration;

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_authority::AuthorityEffect;
use pilotage_protocol::{
    ButtonEdge, ClientHello, ControlPayload, Generation, LeaseRequest, LogicalAxisId,
    LogicalButtonId, ScopeId, ScopedControlFrame, SequenceNum, SessionId, VehicleId,
};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

use crate::{
    ClientKey, DomainEnvelope, LinkLossTrigger, OutboundMessage, SessionAction, SessionConfig,
    SessionEngine, SessionOutcome,
};

/// The single motion scope used across the tests.
pub(crate) fn motion() -> ScopeId {
    ScopeId::new("vehicle.motion")
}

/// The single vehicle used across the tests.
pub(crate) const VEHICLE: VehicleId = VehicleId::new(1);

/// A one-vehicle, one-scope adapter capability report.
pub(crate) fn capabilities() -> AdapterCapabilities {
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            deterministic: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VEHICLE,
            scopes: vec![ScopeDescriptor {
                scope: motion(),
                axes: vec![LogicalAxisId::new(0)],
                intents: vec![pilotage_adapter_api::IntentCapability {
                    family: pilotage_protocol::IntentFamily::Velocity,
                    frames: vec![pilotage_protocol::ReferenceFrame::BodyFrd],
                    max_linear: 1.0,
                    max_vertical: 0.0,
                    max_angular: 1.0,
                }],
                actions: vec![pilotage_adapter_api::ActionCapability {
                    action: pilotage_protocol::ActionKind::Arm,
                    mode_targets: vec![],
                }],
                legacy: Some(pilotage_adapter_api::LegacyCommandMap::Velocity {
                    vx: Some(pilotage_adapter_api::LegacyAxisRoute { axis: 0, sign: 1.0 }),
                    vy: None,
                    vz: None,
                    yaw_rate: None,
                    arm_button: Some(0),
                    disarm_button: None,
                    reset_button: None,
                }),
            }],
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
        }],
        adapter_version: "test".to_owned(),
    }
}

/// A 50 ms staleness policy.
pub(crate) fn staleness() -> StalenessPolicy {
    StalenessPolicy::new(Duration::from_millis(50))
}

/// A fresh engine over [`capabilities`] with protocol floor 1.
pub(crate) fn engine() -> SessionEngine {
    SessionEngine::new(
        capabilities(),
        staleness(),
        SessionConfig::new(1, "host-test"),
    )
}

/// Drives a `ClientHello` for `client` and returns the assigned session id.
pub(crate) fn welcome(engine: &mut SessionEngine, client: ClientKey) -> SessionId {
    let outcome = engine.handle_client_message(
        client,
        crate::DomainEnvelope::Hello(ClientHello {
            protocol_version: 1,
            client_name: "unit".to_owned(),
            join_token: Vec::new(),
        }),
        MonoTimestamp::from_nanos(0),
    );
    match outcome.actions.as_slice() {
        [
            crate::SessionAction::SendToClient {
                envelope: crate::OutboundMessage::Welcome(welcome),
                ..
            },
        ] => welcome.session,
        other => panic!("expected a single welcome, got {other:?}"),
    }
}

/// A control frame for the motion scope at `generation`/`sequence`, sampled at
/// `sampled_at`.
pub(crate) fn frame(
    session: SessionId,
    generation: Generation,
    sequence: SequenceNum,
    sampled_at: MonoTimestamp,
) -> ScopedControlFrame {
    ScopedControlFrame {
        session,
        vehicle: VEHICLE,
        scope: motion(),
        generation,
        sequence,
        sampled_at,
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload {
            axes: vec![(LogicalAxisId::new(0), 0.25)],
            edges: Vec::new(),
        },
        intent: None,
        actions: vec![],
    }
}

/// An engine whose holder-silence window is `silence`, so deadlines land at
/// small, readable nanosecond values.
pub(crate) fn engine_with_silence(silence: Duration) -> SessionEngine {
    SessionEngine::new(
        capabilities(),
        staleness(),
        SessionConfig::new(1, "host-test").with_holder_silence(silence),
    )
}

/// Grants the motion lease to `client` at `now`, returning the granted
/// generation the holder must fence its frames with.
pub(crate) fn grant(engine: &mut SessionEngine, client: ClientKey, now: u64) -> Generation {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(now),
    );
    match outcome.actions.last() {
        Some(SessionAction::SendToClient {
            envelope: OutboundMessage::LeaseResponse(response),
            ..
        }) if response.granted => response.generation,
        other => panic!("expected a granted lease, got {other:?}"),
    }
}

/// Count of `EngageLinkLoss { policy: Neutralize }` actions for [`VEHICLE`].
pub(crate) fn engaged_neutralize(outcome: &SessionOutcome) -> usize {
    outcome
        .actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                SessionAction::EngageLinkLoss {
                    vehicle,
                    policy: LinkLossPolicy::Neutralize,
                    ..
                } if *vehicle == VEHICLE
            )
        })
        .count()
}

/// The trigger of the first `EngageLinkLoss` in the outcome, if any.
pub(crate) fn engage_trigger(outcome: &SessionOutcome) -> Option<LinkLossTrigger> {
    outcome.actions.iter().find_map(|action| match action {
        SessionAction::EngageLinkLoss { trigger, .. } => Some(*trigger),
        _ => None,
    })
}

/// Count of `ClearLinkLoss` actions for [`VEHICLE`].
pub(crate) fn cleared(outcome: &SessionOutcome) -> usize {
    outcome
        .actions
        .iter()
        .filter(
            |action| matches!(action, SessionAction::ClearLinkLoss { vehicle, .. } if *vehicle == VEHICLE),
        )
        .count()
}

/// Whether the outcome broadcast a `HolderLinkLost` authority effect.
pub(crate) fn link_lost(outcome: &SessionOutcome) -> bool {
    outcome.actions.iter().any(|action| {
        matches!(
            action,
            SessionAction::Broadcast {
                envelope: OutboundMessage::Authority(AuthorityEffect::HolderLinkLost { .. }),
            }
        )
    })
}

/// A control frame whose payload demonstrates neutral input (axis at
/// center, no edges) — the recovery activation condition.
pub(crate) fn neutral_frame(
    session: SessionId,
    generation: Generation,
    sequence: SequenceNum,
    sampled_at: MonoTimestamp,
) -> ScopedControlFrame {
    let mut f = frame(session, generation, sequence, sampled_at);
    f.payload = ControlPayload {
        axes: vec![(LogicalAxisId::new(0), 0.0)],
        edges: Vec::new(),
    };
    f
}

/// A control frame carrying only a pressed button edge — alive traffic
/// that is neither setpoint freshness nor a neutral demonstration.
pub(crate) fn edge_only_frame(
    session: SessionId,
    generation: Generation,
    sequence: SequenceNum,
    sampled_at: MonoTimestamp,
) -> ScopedControlFrame {
    let mut f = frame(session, generation, sequence, sampled_at);
    f.payload = ControlPayload {
        axes: Vec::new(),
        edges: vec![(LogicalButtonId::new(0), ButtonEdge::Pressed)],
    };
    f
}

/// The profile-activation traceability record (INPUT-01): an announced
/// activation is retrievable against the session for telemetry/evidence,
/// a re-announcement replaces it, and a pre-handshake announcement closes
/// the connection like any other pre-welcome traffic.
#[test]
fn profile_activation_is_recorded_for_evidence() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    assert!(engine.active_profile(client).is_none());

    // Announcing before the handshake closes the connection.
    let premature = engine.handle_client_message(
        client,
        DomainEnvelope::ProfileActivation(pilotage_protocol::ProfileActivation {
            session: SessionId::new(0),
            profile_id: "builtin.gimbal.default".to_owned(),
            profile_revision: 3,
            activation_revision: 1,
            digest: [0xAB; 32],
        }),
        MonoTimestamp::from_nanos(1),
    );
    assert!(
        premature
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::CloseClient { .. })),
        "pre-welcome activation closes: {:?}",
        premature.actions
    );

    let session = welcome(&mut engine, client);
    let announced = engine.handle_client_message(
        client,
        DomainEnvelope::ProfileActivation(pilotage_protocol::ProfileActivation {
            session,
            profile_id: "builtin.gimbal.default".to_owned(),
            profile_revision: 3,
            activation_revision: 1,
            digest: [0xAB; 32],
        }),
        MonoTimestamp::from_nanos(2),
    );
    assert!(announced.actions.is_empty(), "recording needs no reply");
    let recorded = engine.active_profile(client).expect("activation recorded");
    assert_eq!(recorded.profile_id, "builtin.gimbal.default");
    assert_eq!(recorded.activation_revision, 1);
    assert_eq!(recorded.digest, [0xAB; 32]);

    // A later activation (profile switch) replaces the record.
    let switched = engine.handle_client_message(
        client,
        DomainEnvelope::ProfileActivation(pilotage_protocol::ProfileActivation {
            session,
            profile_id: "user.custom".to_owned(),
            profile_revision: 9,
            activation_revision: 2,
            digest: [0xCD; 32],
        }),
        MonoTimestamp::from_nanos(3),
    );
    assert!(switched.actions.is_empty());
    let replaced = engine.active_profile(client).expect("record replaced");
    assert_eq!(replaced.activation_revision, 2);
    assert_eq!(replaced.digest, [0xCD; 32]);
}

/// The ServerWelcome advertisement carries the adapter's typed capability
/// (CTRL-01): a client can only scale by the advertised envelope if the
/// projection actually forwards it — an empty advertisement silently
/// disables typed control (the client fails closed).
#[test]
fn welcome_advertises_the_typed_capability() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Hello(pilotage_protocol::ClientHello {
            protocol_version: 1,
            client_name: "test".to_owned(),
            join_token: vec![],
        }),
        MonoTimestamp::from_nanos(0),
    );
    let welcome = outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::Welcome(welcome),
                ..
            } => Some(welcome),
            _ => None,
        })
        .expect("welcome sent");
    let scope = &welcome.host_capabilities.vehicles[0].scopes[0];
    let intent = scope.intents.first().expect("velocity intent advertised");
    assert_eq!(
        intent.family,
        pilotage_protocol::wire::IntentFamily::Velocity as i32
    );
    assert_eq!(intent.max_linear, 1.0);
    assert_eq!(intent.max_angular, 1.0);
    assert_eq!(
        intent.frames,
        vec![pilotage_protocol::wire::ReferenceFrame::BodyFrd as i32]
    );
    let action = scope.actions.first().expect("arm action advertised");
    assert_eq!(
        action.action,
        pilotage_protocol::wire::ControlAction::Arm as i32
    );
}

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

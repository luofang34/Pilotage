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

use core::time::Duration;

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_protocol::{
    ClientHello, ControlPayload, Generation, LogicalAxisId, ScopeId, ScopedControlFrame,
    SequenceNum, SessionId, VehicleId,
};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

use crate::{ClientKey, SessionConfig, SessionEngine};

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
    }
}

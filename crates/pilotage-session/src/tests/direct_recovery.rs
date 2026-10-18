//! Recovery on the direct-flight scope: after a velocity→direct scheme
//! transfer engages the flight group's latch, a body-rate frame at zero
//! rates and centre thrust is the neutral demonstration that clears it.

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, IntentCapability, LinkLossPolicy, ScopeDescriptor,
    VehicleDescriptor,
};
use pilotage_protocol::{
    BodyRateIntent, ControlIntent, ControlPayload, Generation, IntentFamily, LeaseRelease,
    LeaseRequest, ReferenceFrame, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

use super::{VEHICLE, welcome};
use crate::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
};

const MOTION: &str = "vehicle.motion";
const DIRECT: &str = "vehicle.motion.direct";

fn capability(family: IntentFamily, frames: Vec<ReferenceFrame>) -> IntentCapability {
    IntentCapability {
        family,
        frames,
        max_linear: 1.0,
        max_vertical: 0.0,
        max_angular: 3.5,
        max_yaw_rate: 0.0,
    }
}

/// A multicopter whose velocity and direct scopes share one authority group.
fn multicopter_engine() -> SessionEngine {
    let scope = |name: &str, intents: Vec<IntentCapability>| ScopeDescriptor {
        authority_group: Some(MOTION.to_owned()),
        scope: ScopeId::new(name),
        axes: vec![],
        intents,
        actions: vec![],
        legacy: None,
    };
    let capabilities = AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            deterministic: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VEHICLE,
            class: pilotage_protocol::VehicleClass::Multicopter,
            display_name: "test quad".to_owned(),
            scopes: vec![
                scope(
                    MOTION,
                    vec![capability(
                        IntentFamily::Velocity,
                        vec![ReferenceFrame::BodyFrd],
                    )],
                ),
                scope(DIRECT, vec![capability(IntentFamily::BodyRate, vec![])]),
            ],
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
        }],
        adapter_version: "test".to_owned(),
        control_feel: None,
    };
    SessionEngine::new(
        capabilities,
        StalenessPolicy::new(core::time::Duration::from_millis(50)),
        SessionConfig::new(1, "host-test"),
    )
}

/// Activates a control profile, as every flying client does first.
fn announce(engine: &mut SessionEngine, client: ClientKey, session: SessionId) {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::ProfileActivation(pilotage_protocol::ProfileActivation {
            session,
            profile_id: "builtin.flight.default".to_owned(),
            profile_revision: 1,
            activation_revision: 1,
            digest: [0x11; 32],
            device_profile_id: String::new(),
            device_profile_revision: 0,
            device_digest: [0; 32],
        }),
        MonoTimestamp::from_nanos(1),
    );
    assert!(
        matches!(
            outcome.actions.as_slice(),
            [SessionAction::ActivationAccepted { .. }]
        ),
        "got {:?}",
        outcome.actions
    );
}

fn lease(engine: &mut SessionEngine, client: ClientKey, scope: &str, at: u64) -> Generation {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(LeaseRequest {
            vehicle: VEHICLE,
            scope: ScopeId::new(scope),
        }),
        MonoTimestamp::from_nanos(at),
    );
    outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            } if response.granted => Some(response.generation),
            _ => None,
        })
        .expect("a granted lease")
}

fn body_rate_frame(
    session: SessionId,
    generation: Generation,
    sequence: u32,
    at: u64,
    intent: BodyRateIntent,
) -> ScopedControlFrame {
    ScopedControlFrame {
        session,
        vehicle: VEHICLE,
        scope: ScopeId::new(DIRECT),
        generation,
        sequence: SequenceNum::new(sequence),
        sampled_at: MonoTimestamp::from_nanos(at),
        profile_revision: 1,
        activation_revision: 1,
        payload: ControlPayload::default(),
        intent: Some(ControlIntent::BodyRate(intent)),
        actions: vec![],
        action_ids: vec![],
    }
}

fn clears(actions: &[SessionAction]) -> bool {
    actions.iter().any(|action| {
        matches!(action, SessionAction::ClearLinkLoss { scope, .. } if scope.as_str() == MOTION)
    })
}

#[test]
fn a_neutral_body_rate_frame_clears_the_latch_a_scheme_transfer_engaged() {
    let mut engine = multicopter_engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session);
    lease(&mut engine, client, MOTION, 2);
    let released = engine.handle_client_message(
        client,
        DomainEnvelope::Release(LeaseRelease {
            vehicle: VEHICLE,
            scope: ScopeId::new(MOTION),
        }),
        MonoTimestamp::from_nanos(3),
    );
    assert!(
        released.actions.iter().any(|action| matches!(
            action,
            SessionAction::EngageLinkLoss { scope, .. } if scope.as_str() == MOTION
        )),
        "the transfer engages the group latch: {:?}",
        released.actions
    );
    let generation = lease(&mut engine, client, DIRECT, 4);

    let rolling = BodyRateIntent {
        roll_rate: 1.0,
        pitch_rate: 0.0,
        yaw_rate: 0.0,
        thrust: 0.5,
    };
    let deflected = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(body_rate_frame(session, generation, 1, 5, rolling)),
        MonoTimestamp::from_nanos(5),
    );
    assert!(!clears(&deflected.actions), "a 1 rad/s roll is not neutral");

    let neutral = BodyRateIntent {
        roll_rate: 0.0,
        pitch_rate: 0.0,
        yaw_rate: 0.0,
        thrust: 0.5,
    };
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(body_rate_frame(session, generation, 2, 6, neutral)),
        MonoTimestamp::from_nanos(6),
    );
    assert!(
        clears(&outcome.actions),
        "zero rates at centre thrust clear the latch: {:?}",
        outcome.actions
    );
}

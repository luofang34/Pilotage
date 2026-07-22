//! The exclusive-authority group (CTRL-01): scopes that drive the same
//! actuator lease, fence, watch, latch, and recover as ONE authority.
//! `vehicle.motion` and `vehicle.motion.direct` are the motivating pair —
//! they must never be held apart, their generations share one domain, and
//! a scope handover can never leave an orphaned sibling latch.

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, IntentCapability, LinkLossPolicy, ScopeDescriptor,
    VehicleDescriptor,
};
use pilotage_protocol::{
    ControlIntent, ControlPayload, Generation, IntentFamily, LeaseDenialReason, LeaseRelease,
    LeaseRequest, ReferenceFrame, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
    VelocityIntent,
};
use pilotage_timing::{MonoTimestamp, StalenessPolicy};

use super::{VEHICLE, welcome};
use crate::{
    ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig, SessionEngine,
};

const MOTION: &str = "vehicle.motion";
const DIRECT: &str = "vehicle.motion.direct";

/// Two flight scopes sharing one authority group, velocity-family both (the
/// family split is the gate's business; the group is authority's).
fn grouped_engine() -> SessionEngine {
    let scope = |name: &str| ScopeDescriptor {
        authority_group: Some(MOTION.to_owned()),
        scope: ScopeId::new(name),
        axes: vec![],
        intents: vec![IntentCapability {
            family: IntentFamily::Velocity,
            frames: vec![ReferenceFrame::BodyFrd],
            max_linear: 1.0,
            max_vertical: 0.0,
            max_angular: 1.0,
            max_yaw_rate: 0.0,
        }],
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
            scopes: vec![scope(MOTION), scope(DIRECT)],
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
        }],
        adapter_version: "test".to_owned(),
    };
    SessionEngine::new(
        capabilities,
        StalenessPolicy::new(core::time::Duration::from_millis(50)),
        SessionConfig::new(1, "host-test"),
    )
}

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
    assert!(outcome.actions.is_empty());
}

fn lease(
    engine: &mut SessionEngine,
    client: ClientKey,
    scope: &str,
    at: u64,
) -> Result<Generation, LeaseDenialReason> {
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
            } => Some(if response.granted {
                Ok(response.generation)
            } else {
                Err(response.reason.expect("denials carry a reason"))
            }),
            _ => None,
        })
        .expect("a lease response")
}

fn release(engine: &mut SessionEngine, client: ClientKey, scope: &str, at: u64) {
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Release(LeaseRelease {
            vehicle: VEHICLE,
            scope: ScopeId::new(scope),
        }),
        MonoTimestamp::from_nanos(at),
    );
    assert!(!outcome.actions.is_empty());
}

fn neutral_typed_frame(
    session: SessionId,
    scope: &str,
    generation: Generation,
    sequence: u32,
    at: u64,
) -> ScopedControlFrame {
    ScopedControlFrame {
        session,
        vehicle: VEHICLE,
        scope: ScopeId::new(scope),
        generation,
        sequence: SequenceNum::new(sequence),
        sampled_at: MonoTimestamp::from_nanos(at),
        profile_revision: 1,
        activation_revision: 1,
        payload: ControlPayload::default(),
        intent: Some(ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            yaw_rate: 0.0,
        })),
        actions: vec![],
        action_ids: vec![],
    }
}

#[test]
fn sibling_scopes_are_never_held_by_different_clients() {
    let mut engine = grouped_engine();
    let first = ClientKey::new(1);
    let second = ClientKey::new(2);
    welcome(&mut engine, first);
    welcome(&mut engine, second);
    lease(&mut engine, first, MOTION, 2).expect("first holder");
    let denied = lease(&mut engine, second, DIRECT, 3).expect_err("the sibling is one authority");
    assert_eq!(denied, LeaseDenialReason::AlreadyHeld);
}

#[test]
fn the_group_shares_one_generation_domain_across_a_handover() {
    let mut engine = grouped_engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    let motion_generation = lease(&mut engine, client, MOTION, 2).expect("granted");
    release(&mut engine, client, MOTION, 3);
    let direct_generation = lease(&mut engine, client, DIRECT, 4).expect("granted");
    assert!(
        direct_generation > motion_generation,
        "the sibling grant is STRICTLY newer in the same domain \
         ({direct_generation:?} vs {motion_generation:?}) — a delayed frame \
         or action bound to the old grant can never satisfy the new fence"
    );
}

#[test]
fn a_scope_handover_leaves_no_orphaned_sibling_latch() {
    let mut engine = grouped_engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session);
    lease(&mut engine, client, MOTION, 2).expect("granted");

    // Releasing the motion scope engages the GROUP's link-loss latch.
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
        "the release engages the group latch: {:?}",
        released.actions
    );

    // The handover acquires the SIBLING scope; a neutral demonstration on
    // it must clear the shared latch — the old member scope cannot stay
    // engaged behind the swap.
    let generation = lease(&mut engine, client, DIRECT, 4).expect("granted");
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(neutral_typed_frame(session, DIRECT, generation, 0, 5)),
        MonoTimestamp::from_nanos(6),
    );
    assert!(
        outcome.actions.iter().any(|action| matches!(
            action,
            SessionAction::ClearLinkLoss { scope, .. } if scope.as_str() == MOTION
        )),
        "the sibling's neutral demonstration clears the ONE group latch: {:?}",
        outcome.actions
    );
}

#[test]
fn the_group_holder_may_drive_through_either_member_scope() {
    // The group IS the authority: the gate still types each member's
    // commands by that member's own advertisement, but holder identity and
    // fencing resolve at the group.
    let mut engine = grouped_engine();
    let client = ClientKey::new(1);
    let session = welcome(&mut engine, client);
    announce(&mut engine, client, session);
    let generation = lease(&mut engine, client, DIRECT, 2).expect("granted");
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(neutral_typed_frame(session, MOTION, generation, 0, 3)),
        MonoTimestamp::from_nanos(4),
    );
    assert!(
        outcome
            .actions
            .iter()
            .any(|action| matches!(action, SessionAction::ApplyToAdapter { .. })),
        "got {:?}",
        outcome.actions
    );
}

//! Disconnect: a departing holder's scopes are released via the authority
//! engine's link-loss path (ADR-0010).

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, LinkLossPolicy, ScopeDescriptor, VehicleDescriptor,
};
use pilotage_authority::AuthorityEffect;
use pilotage_protocol::{LeaseRequest, LogicalAxisId, ScopeId};
use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engine, motion, staleness, welcome};
use crate::{ClientKey, DomainEnvelope, OutboundMessage, SessionAction, SessionConfig};

fn lease_to(engine: &mut crate::SessionEngine, client: ClientKey) {
    let _granted = engine.handle_client_message(
        client,
        DomainEnvelope::Lease(pilotage_protocol::LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(1),
    );
}

#[test]
fn disconnect_releases_held_scope_and_broadcasts_link_lost() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    lease_to(&mut engine, client);

    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Disconnect,
        MonoTimestamp::from_nanos(5),
    );
    // The link-loss path emits LinkStateChanged then HolderLinkLost, each as an
    // authority broadcast.
    let lost = outcome.actions.iter().any(|action| {
        matches!(
            action,
            SessionAction::Broadcast {
                envelope: OutboundMessage::Authority(AuthorityEffect::HolderLinkLost { .. }),
            }
        )
    });
    assert!(
        lost,
        "expected a HolderLinkLost broadcast, got {:?}",
        outcome.actions
    );
    // Releasing one scope is well under the cap, so nothing is dropped.
    assert_eq!(outcome.dropped, 0);
}

#[test]
fn disconnect_of_non_holder_releases_nothing() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Disconnect,
        MonoTimestamp::from_nanos(5),
    );
    assert!(
        outcome.actions.is_empty(),
        "expected no releases, got {:?}",
        outcome.actions
    );
}

#[test]
fn disconnect_of_unknown_client_is_a_no_op() {
    let mut engine = engine();
    let outcome = engine.handle_client_message(
        ClientKey::new(99),
        DomainEnvelope::Disconnect,
        MonoTimestamp::from_nanos(5),
    );
    assert!(outcome.actions.is_empty());
}

#[test]
fn scope_is_grantable_again_after_holder_disconnects() {
    let mut engine = engine();
    let first = ClientKey::new(1);
    let second = ClientKey::new(2);
    welcome(&mut engine, first);
    lease_to(&mut engine, first);
    let _released = engine.handle_client_message(
        first,
        DomainEnvelope::Disconnect,
        MonoTimestamp::from_nanos(5),
    );

    welcome(&mut engine, second);
    let outcome = engine.handle_client_message(
        second,
        DomainEnvelope::Lease(pilotage_protocol::LeaseRequest {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        MonoTimestamp::from_nanos(6),
    );
    match outcome.actions.last() {
        Some(SessionAction::SendToClient {
            envelope: OutboundMessage::LeaseResponse(response),
            ..
        }) => assert!(response.granted, "re-grant after release should succeed"),
        other => panic!("expected a granted lease, got {other:?}"),
    }
}

/// A one-vehicle adapter exposing `scopes` distinct motion scopes.
fn many_scope_capabilities(scopes: usize) -> AdapterCapabilities {
    let scope_descriptors = (0..scopes)
        .map(|i| ScopeDescriptor {
            scope: ScopeId::new(format!("vehicle.motion.{i}")),
            axes: vec![LogicalAxisId::new(0)],
        })
        .collect();
    AdapterCapabilities {
        execution: ExecutionMode {
            real_time: true,
            deterministic: true,
            ..ExecutionMode::default()
        },
        vehicles: vec![VehicleDescriptor {
            id: VEHICLE,
            scopes: scope_descriptors,
            link_loss_actions: vec![LinkLossPolicy::Neutralize],
        }],
        adapter_version: "test".to_owned(),
    }
}

#[test]
fn disconnect_dropping_link_lost_broadcasts_is_reported_not_silent() {
    // Hold more scopes than the per-call cap admits, so a single disconnect's
    // release fan-out exceeds the cap. The finding: without an observable drop
    // count, authority state advances while `HolderLinkLost` broadcasts are
    // silently truncated and clients never learn the released scopes.
    let cap = 4;
    let scopes = 8;
    let mut engine = crate::SessionEngine::new(
        many_scope_capabilities(scopes),
        staleness(),
        SessionConfig::new(1, "host-test").with_max_actions_per_call(cap),
    );
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    for i in 0..scopes {
        let _granted = engine.handle_client_message(
            client,
            DomainEnvelope::Lease(LeaseRequest {
                vehicle: VEHICLE,
                scope: ScopeId::new(format!("vehicle.motion.{i}")),
            }),
            MonoTimestamp::from_nanos(1),
        );
    }

    let outcome = engine.handle_client_message(
        client,
        DomainEnvelope::Disconnect,
        MonoTimestamp::from_nanos(5),
    );
    // The cap truncated the ordinary actions (broadcasts), while the safety
    // lane still delivered the vehicle's single link-loss engagement above
    // the cap — a safety enactment is never droppable behind the broadcast
    // cap.
    let engagements = outcome
        .actions
        .iter()
        .filter(|a| matches!(a, SessionAction::EngageLinkLoss { .. }))
        .count();
    assert_eq!(engagements, 1, "one engagement per vehicle loss");
    assert_eq!(
        outcome.actions.len(),
        cap + engagements,
        "ordinary actions stay capped: {:?}",
        outcome.actions
    );
    // ...and the truncation is reported, not silent (the finding's contract).
    assert!(
        outcome.dropped > 0,
        "dropped link-lost broadcasts must be observable, got {}",
        outcome.dropped
    );
}

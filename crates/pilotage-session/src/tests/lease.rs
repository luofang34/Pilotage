//! Lease: grant emits a granted response plus an authority broadcast; a
//! duplicate request by the holder re-affirms; another principal is denied.

use pilotage_authority::AuthorityEffect;
use pilotage_protocol::{LeaseDenialReason, LeaseRequest, ScopeId, VehicleId};
use pilotage_timing::MonoTimestamp;

use super::{VEHICLE, engine, motion, welcome};
use crate::{ClientKey, DomainEnvelope, OutboundMessage, SessionAction};

fn lease(vehicle: VehicleId, scope: ScopeId) -> DomainEnvelope {
    DomainEnvelope::Lease(LeaseRequest { vehicle, scope })
}

fn request_lease(
    engine: &mut crate::SessionEngine,
    client: ClientKey,
    vehicle: VehicleId,
    scope: ScopeId,
) -> Vec<SessionAction> {
    engine
        .handle_client_message(client, lease(vehicle, scope), MonoTimestamp::from_nanos(10))
        .actions
}

#[test]
fn grant_emits_broadcast_and_granted_response() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    let actions = request_lease(&mut engine, client, VEHICLE, motion());

    // A broadcast authority grant precedes the unicast lease response.
    assert!(matches!(
        actions.first(),
        Some(SessionAction::Broadcast {
            envelope: OutboundMessage::Authority(AuthorityEffect::ScopeLeaseGranted { .. }),
        })
    ));
    match actions.last() {
        Some(SessionAction::SendToClient {
            envelope: OutboundMessage::LeaseResponse(response),
            ..
        }) => {
            assert!(response.granted);
            assert!(response.reason.is_none());
            // Grant advances the generation off the registered zero.
            assert_eq!(response.generation.as_u64(), 1);
        }
        other => panic!("expected a granted lease response, got {other:?}"),
    }
}

#[test]
fn duplicate_request_by_holder_reaffirms_without_new_broadcast() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    request_lease(&mut engine, client, VEHICLE, motion());
    let actions = request_lease(&mut engine, client, VEHICLE, motion());
    // No authority effect: the standing grant is simply re-reported.
    match actions.as_slice() {
        [
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            },
        ] => {
            assert!(response.granted);
            assert_eq!(response.generation.as_u64(), 1);
        }
        other => panic!("expected a lone re-affirmed grant, got {other:?}"),
    }
}

#[test]
fn request_by_another_principal_is_denied_already_held() {
    let mut engine = engine();
    let holder = ClientKey::new(1);
    let other = ClientKey::new(2);
    welcome(&mut engine, holder);
    welcome(&mut engine, other);
    request_lease(&mut engine, holder, VEHICLE, motion());
    let actions = request_lease(&mut engine, other, VEHICLE, motion());
    match actions.as_slice() {
        [
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            },
        ] => {
            assert!(!response.granted);
            assert_eq!(response.reason, Some(LeaseDenialReason::AlreadyHeld));
            assert_eq!(response.generation.as_u64(), 1);
        }
        other => panic!("expected an already-held denial, got {other:?}"),
    }
    // The holder must be unchanged: the original holder still re-affirms its
    // own standing grant at the same generation, never a fresh grant that
    // would imply `other`'s denied request mutated anything.
    let reaffirm = request_lease(&mut engine, holder, VEHICLE, motion());
    match reaffirm.as_slice() {
        [
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            },
        ] => {
            assert!(response.granted);
            assert_eq!(response.reason, None);
            assert_eq!(response.generation.as_u64(), 1);
        }
        other => panic!("expected the original holder's grant reaffirmed, got {other:?}"),
    }
}

#[test]
fn unknown_scope_is_denied() {
    let mut engine = engine();
    let client = ClientKey::new(1);
    welcome(&mut engine, client);
    let actions = request_lease(&mut engine, client, VEHICLE, ScopeId::new("vehicle.ghost"));
    match actions.as_slice() {
        [
            SessionAction::SendToClient {
                envelope: OutboundMessage::LeaseResponse(response),
                ..
            },
        ] => {
            assert!(!response.granted);
            assert_eq!(response.reason, Some(LeaseDenialReason::UnknownScope));
        }
        other => panic!("expected an unknown-scope denial, got {other:?}"),
    }
}

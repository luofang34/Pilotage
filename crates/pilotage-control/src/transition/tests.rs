use pilotage_protocol::wire;

use super::*;

const VEHICLE: u64 = 7;
const PRINCIPAL: u64 = 42;

fn route(motion_scope: &str) -> AuthorityRoute {
    AuthorityRoute {
        vehicle_id: VEHICLE,
        principal_id: PRINCIPAL,
        motion_scope: motion_scope.to_owned(),
    }
}

fn ids(vehicle: u64, scope: &str) -> (Option<wire::VehicleId>, Option<wire::ScopeId>) {
    (
        Some(wire::VehicleId { value: vehicle }),
        Some(wire::ScopeId {
            value: scope.to_owned(),
        }),
    )
}

fn fence(value: u64) -> Option<wire::Generation> {
    Some(wire::Generation { value })
}

fn response(vehicle: u64, scope: &str, granted: bool, generation: u64) -> wire::LeaseResponse {
    let (vehicle, scope) = ids(vehicle, scope);
    wire::LeaseResponse {
        vehicle,
        scope,
        granted,
        generation: fence(generation),
        ..wire::LeaseResponse::default()
    }
}

fn rejection(scope: &str, reason: wire::FrameRejectionReason, current: u64) -> wire::FrameRejected {
    let (vehicle, scope) = ids(VEHICLE, scope);
    wire::FrameRejected {
        vehicle,
        scope,
        reason: reason as i32,
        current_generation: fence(current),
        ..wire::FrameRejected::default()
    }
}

#[test]
fn a_grant_and_a_denial_of_our_request_are_transitions() {
    let route = route(MOTION_SCOPE);
    assert_eq!(
        route.lease_response(&response(VEHICLE, MOTION_SCOPE, true, 3)),
        Some((
            AuthorityScope::Motion,
            AuthorityEvent::LeaseGranted { generation: 3 }
        ))
    );
    assert_eq!(
        route.lease_response(&response(VEHICLE, GIMBAL_SCOPE, false, 0)),
        Some((AuthorityScope::Gimbal, AuthorityEvent::LeaseDenied))
    );
}

#[test]
fn another_vehicle_or_motion_member_is_not_ours() {
    let route = route(MOTION_SCOPE);
    assert_eq!(
        route.lease_response(&response(8, MOTION_SCOPE, true, 3)),
        None
    );
    assert_eq!(
        route.lease_response(&response(VEHICLE, DIRECT_SCOPE, true, 3)),
        None
    );
    assert_eq!(
        route.lease_response(&response(VEHICLE, "payload.drop", true, 3)),
        None
    );
}

#[test]
fn a_release_acknowledgement_carries_the_fence() {
    let (vehicle, scope) = ids(VEHICLE, GIMBAL_SCOPE);
    let released = wire::LeaseReleased {
        vehicle,
        scope,
        released: true,
        generation: fence(5),
    };
    assert_eq!(
        route(MOTION_SCOPE).lease_released(&released),
        Some((
            AuthorityScope::Gimbal,
            AuthorityEvent::LeaseReleased { generation: 5 }
        ))
    );
}

fn revoked(principal: u64, scope: &str) -> wire::AuthorityEvent {
    let (vehicle, scope) = ids(VEHICLE, scope);
    wire::AuthorityEvent {
        event: Some(wire::authority_event::Event::ScopeLeaseRevoked(
            wire::ScopeLeaseRevoked {
                principal: Some(wire::PrincipalId { value: principal }),
                vehicle,
                scope,
                generation: fence(9),
                ..wire::ScopeLeaseRevoked::default()
            },
        )),
    }
}

#[test]
fn only_our_principal_revocations_reach_the_table() {
    // The direct member flies under the velocity member's group.
    let route = route(DIRECT_SCOPE);
    assert_eq!(
        route.authority_event(&revoked(PRINCIPAL, MOTION_SCOPE)),
        Some((
            AuthorityScope::Motion,
            AuthorityEvent::Revoked { generation: 9 }
        ))
    );
    assert_eq!(route.authority_event(&revoked(43, MOTION_SCOPE)), None);
}

#[test]
fn link_loss_recovery_is_the_motion_group_only() {
    let cleared = |scope: &str| {
        let (vehicle, scope) = ids(VEHICLE, scope);
        wire::LinkLossCleared {
            vehicle,
            scope,
            generation: fence(4),
        }
    };
    let route = route(DIRECT_SCOPE);
    assert_eq!(
        route.link_loss_cleared(&cleared(MOTION_SCOPE)),
        Some((
            AuthorityScope::Motion,
            AuthorityEvent::LinkLossCleared { generation: 4 }
        ))
    );
    assert_eq!(route.link_loss_cleared(&cleared(GIMBAL_SCOPE)), None);
}

#[test]
fn rejections_report_an_idle_uplink_or_a_lost_fence() {
    use wire::FrameRejectionReason as Reason;
    let route = route(MOTION_SCOPE);
    assert_eq!(
        route.frame_rejected(&rejection(MOTION_SCOPE, Reason::UplinkIdle, 2)),
        Some((AuthorityScope::Motion, AuthorityEvent::UplinkIdle))
    );
    assert_eq!(
        route.frame_rejected(&rejection(GIMBAL_SCOPE, Reason::StaleGeneration, 6)),
        Some((
            AuthorityScope::Gimbal,
            AuthorityEvent::Revoked { generation: 6 }
        ))
    );
    assert_eq!(
        route.frame_rejected(&rejection(MOTION_SCOPE, Reason::NoHolder, 6)),
        Some((
            AuthorityScope::Motion,
            AuthorityEvent::Revoked { generation: 6 }
        ))
    );
    assert_eq!(
        route.frame_rejected(&rejection(MOTION_SCOPE, Reason::TooOld, 6)),
        None
    );
    assert_eq!(
        route.frame_rejected(&rejection(LIFECYCLE_SCOPE, Reason::NoHolder, 6)),
        None
    );
}

#[test]
fn an_action_result_feeds_the_motion_slot_only() {
    let route = route(MOTION_SCOPE);
    assert_eq!(
        route.action_result(MOTION_SCOPE, wire::ControlAction::Arm, true),
        Some((
            AuthorityScope::Motion,
            AuthorityEvent::ActionResult {
                action: wire::ControlAction::Arm as u32,
                accepted: true,
            }
        ))
    );
    assert_eq!(
        route.action_result(LIFECYCLE_SCOPE, wire::ControlAction::SimReset, true),
        None
    );
}

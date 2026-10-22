//! Which reliable-stream messages are authority transitions of this client's
//! table, and which transition each one is.
//!
//! The host speaks about every vehicle, scope, and principal on one stream;
//! the authority table must hear only its own scopes on its own vehicle, and
//! only the lease traffic addressed to this principal. Every operator shell
//! routes through these functions so the browser and native clients file the
//! same message under the same slot.

use pilotage_protocol::wire;

use crate::authority::{AuthorityEvent, AuthorityScope};
use crate::plan::{GIMBAL_SCOPE, MOTION_SCOPE};

/// The motion group's direct-flight member: leased on its own, fenced with
/// the velocity member as one motion authority.
pub use pilotage_protocol::DIRECT_SCOPE;
/// The on-demand simulator lifecycle scope.
pub const LIFECYCLE_SCOPE: &str = "sim.lifecycle";

/// A frame refused because the adapter admitted it but cannot enact it
/// before an arm (`FRAME_REJECTION_REASON_UPLINK_IDLE`).
const REJECTED_UPLINK_IDLE: i32 = wire::FrameRejectionReason::UplinkIdle as i32;
/// A frame refused under an older generation, or with no holder at all: the
/// sender no longer holds what it fenced on.
const REJECTED_FENCE: [i32; 2] = [
    wire::FrameRejectionReason::StaleGeneration as i32,
    wire::FrameRejectionReason::NoHolder as i32,
];

/// One client's view of the stream: its vehicle, its principal, and which
/// motion-group member it currently flies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityRoute {
    /// The vehicle this client controls.
    pub vehicle_id: u64,
    /// The principal the host assigned this connection at admission.
    pub principal_id: u64,
    /// The motion-group member this client holds or requests.
    pub motion_scope: String,
}

impl AuthorityRoute {
    /// The slot a scope's transitions belong to, or `None` for a scope this
    /// client's table does not follow. The two motion members share one slot.
    #[must_use]
    pub fn slot(&self, scope: &str) -> Option<AuthorityScope> {
        match scope {
            GIMBAL_SCOPE => Some(AuthorityScope::Gimbal),
            LIFECYCLE_SCOPE => Some(AuthorityScope::Lifecycle),
            _ if motion_group(scope) == motion_group(&self.motion_scope) => {
                Some(AuthorityScope::Motion)
            }
            _ => None,
        }
    }

    /// The answer to this client's own lease request: for motion, only the
    /// member it asked for.
    #[must_use]
    pub fn lease_response(
        &self,
        response: &wire::LeaseResponse,
    ) -> Option<(AuthorityScope, AuthorityEvent)> {
        let slot = self.requested(response.vehicle.as_ref(), response.scope.as_ref())?;
        let event = if response.granted {
            AuthorityEvent::LeaseGranted {
                generation: generation(response.generation.as_ref()),
            }
        } else {
            AuthorityEvent::LeaseDenied
        };
        Some((slot, event))
    }

    /// The host's acknowledgement of this client's release: for motion, only
    /// the member it holds.
    #[must_use]
    pub fn lease_released(
        &self,
        released: &wire::LeaseReleased,
    ) -> Option<(AuthorityScope, AuthorityEvent)> {
        let slot = self.requested(released.vehicle.as_ref(), released.scope.as_ref())?;
        Some((
            slot,
            AuthorityEvent::LeaseReleased {
                generation: generation(released.generation.as_ref()),
            },
        ))
    }

    /// A lane closed by an authority event, without denying a fresh request.
    #[must_use]
    pub fn lane_closed(
        &self,
        vehicle: u64,
        scope: &str,
        generation: u64,
    ) -> Option<(AuthorityScope, AuthorityEvent)> {
        if vehicle != self.vehicle_id {
            return None;
        }
        Some((self.slot(scope)?, AuthorityEvent::Revoked { generation }))
    }

    /// A grant or revocation addressed to this principal. Every other
    /// authority event (another principal's lease, a transfer, an override,
    /// a warning) is not a transition of this table.
    #[must_use]
    pub fn authority_event(
        &self,
        event: &wire::AuthorityEvent,
    ) -> Option<(AuthorityScope, AuthorityEvent)> {
        use wire::authority_event::Event;
        let (principal, vehicle, scope, fence, granted) = match event.event.as_ref()? {
            Event::ScopeLeaseGranted(g) => {
                (&g.principal, &g.vehicle, &g.scope, &g.generation, true)
            }
            Event::ScopeLeaseRevoked(r) => {
                (&r.principal, &r.vehicle, &r.scope, &r.generation, false)
            }
            _ => return None,
        };
        if principal.as_ref().map_or(0, |p| p.value) != self.principal_id {
            return None;
        }
        let slot = self.own(vehicle.as_ref(), scope.as_ref())?;
        let generation = generation(fence.as_ref());
        let event = if granted {
            AuthorityEvent::LeaseGranted { generation }
        } else {
            AuthorityEvent::Revoked { generation }
        };
        Some((slot, event))
    }

    /// The host's confirmation that the motion group recovered from a link
    /// loss on this generation.
    #[must_use]
    pub fn link_loss_cleared(
        &self,
        cleared: &wire::LinkLossCleared,
    ) -> Option<(AuthorityScope, AuthorityEvent)> {
        let group = motion_group(&self.motion_scope);
        let ours = cleared.vehicle.as_ref().map_or(0, |v| v.value) == self.vehicle_id
            && scope_name(cleared.scope.as_ref()) == group;
        ours.then(|| {
            let slot = AuthorityScope::Motion;
            (
                slot,
                AuthorityEvent::LinkLossCleared {
                    generation: generation(cleared.generation.as_ref()),
                },
            )
        })
    }

    /// What a rejected frame says about this client's authority: an idle
    /// uplink that needs an arm, or a fence this client no longer holds.
    /// Every other rejection is a diagnostic, not a transition.
    #[must_use]
    pub fn frame_rejected(
        &self,
        rejection: &wire::FrameRejected,
    ) -> Option<(AuthorityScope, AuthorityEvent)> {
        let slot = self.slot(&scope_name(rejection.scope.as_ref()))?;
        let generation = generation(rejection.current_generation.as_ref());
        if rejection.reason == REJECTED_UPLINK_IDLE && slot == AuthorityScope::Motion {
            return Some((slot, AuthorityEvent::UplinkIdle));
        }
        (REJECTED_FENCE.contains(&rejection.reason) && slot != AuthorityScope::Lifecycle)
            .then_some((slot, AuthorityEvent::Revoked { generation }))
    }

    /// The result of an action this client sent under `scope`. The caller
    /// pairs the result with its own record of the action by id, since a
    /// result is only meaningful against the press that caused it.
    #[must_use]
    pub fn action_result(
        &self,
        scope: &str,
        action: wire::ControlAction,
        accepted: bool,
    ) -> Option<(AuthorityScope, AuthorityEvent)> {
        (self.slot(scope)? == AuthorityScope::Motion).then_some((
            AuthorityScope::Motion,
            AuthorityEvent::ActionResult {
                action: action as u32,
                accepted,
            },
        ))
    }

    /// The slot of a scope this client requests by name: the motion member
    /// exactly, since the two members are leased separately.
    fn requested(
        &self,
        vehicle: Option<&wire::VehicleId>,
        scope: Option<&wire::ScopeId>,
    ) -> Option<AuthorityScope> {
        let slot = self.own(vehicle, scope)?;
        let exact = slot != AuthorityScope::Motion || scope_name(scope) == self.motion_scope;
        exact.then_some(slot)
    }

    fn own(
        &self,
        vehicle: Option<&wire::VehicleId>,
        scope: Option<&wire::ScopeId>,
    ) -> Option<AuthorityScope> {
        if vehicle.map_or(0, |v| v.value) != self.vehicle_id {
            return None;
        }
        self.slot(&scope_name(scope))
    }
}

/// The motion group a scope belongs to: the direct member flies under the
/// velocity member's group.
fn motion_group(scope: &str) -> &str {
    if scope == DIRECT_SCOPE {
        MOTION_SCOPE
    } else {
        scope
    }
}

fn scope_name(scope: Option<&wire::ScopeId>) -> String {
    scope.map(|s| s.value.clone()).unwrap_or_default()
}

fn generation(generation: Option<&wire::Generation>) -> u64 {
    generation.map_or(0, |g| g.value)
}

#[cfg(test)]
mod tests;

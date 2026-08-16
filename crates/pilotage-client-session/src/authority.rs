//! The client's mirror of authority state.
//!
//! The host is the authority owner; this mirror only tracks what the host
//! has said, so a shell can render ownership and the engine can tell a
//! grant addressed to this principal from everyone else's traffic.

use std::collections::BTreeMap;

use pilotage_protocol::wire;

/// Holder and fencing generation for one (vehicle, scope), as last stated
/// by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeHolder {
    /// Present holder, absent when the scope is unassigned.
    pub holder_id: Option<u64>,
    /// Fencing generation of the present assignment.
    pub generation: u64,
}

/// Every (vehicle, scope) the host has described, by last statement.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AuthorityMirror {
    holders: BTreeMap<(u64, String), ScopeHolder>,
}

impl AuthorityMirror {
    /// Seeds the mirror from the admission snapshot.
    pub fn seed(&mut self, snapshots: &[wire::ScopeHolderSnapshot]) {
        self.holders.clear();
        for snapshot in snapshots {
            let vehicle = snapshot.vehicle.as_ref().map_or(0, |v| v.value);
            let scope = snapshot
                .scope
                .as_ref()
                .map(|s| s.value.clone())
                .unwrap_or_default();
            self.holders.insert(
                (vehicle, scope),
                ScopeHolder {
                    holder_id: snapshot.holder.as_ref().map(|h| h.value),
                    generation: snapshot.generation.as_ref().map_or(0, |g| g.value),
                },
            );
        }
    }

    /// Applies one authority event. Returns the grant's generation when the
    /// event grants `(vehicle, scope)` to `principal_id`; every other event,
    /// including a revocation, returns `None` — callers watch
    /// [`AuthorityMirror::holder`] for loss.
    pub fn apply(&mut self, event: &wire::AuthorityEvent, principal_id: u64) -> Option<u64> {
        match event.event.as_ref()? {
            wire::authority_event::Event::ScopeLeaseGranted(granted) => {
                let key = scope_key(granted.vehicle.as_ref(), granted.scope.as_ref());
                let holder = granted.principal.as_ref().map(|p| p.value);
                let generation = granted.generation.as_ref().map_or(0, |g| g.value);
                self.holders.insert(
                    key,
                    ScopeHolder {
                        holder_id: holder,
                        generation,
                    },
                );
                (holder == Some(principal_id)).then_some(generation)
            }
            wire::authority_event::Event::ScopeLeaseRevoked(revoked) => {
                let key = scope_key(revoked.vehicle.as_ref(), revoked.scope.as_ref());
                if let Some(entry) = self.holders.get_mut(&key) {
                    entry.holder_id = None;
                    entry.generation = revoked.generation.as_ref().map_or(0, |g| g.value);
                }
                None
            }
            _ => None,
        }
    }

    /// The last-stated holder of one (vehicle, scope).
    #[must_use]
    pub fn holder(&self, vehicle_id: u64, scope: &str) -> Option<&ScopeHolder> {
        self.holders.get(&(vehicle_id, scope.to_owned()))
    }
}

/// The mirror's map key for one event's (vehicle, scope) pair.
fn scope_key(vehicle: Option<&wire::VehicleId>, scope: Option<&wire::ScopeId>) -> (u64, String) {
    (
        vehicle.map_or(0, |v| v.value),
        scope.map(|s| s.value.clone()).unwrap_or_default(),
    )
}

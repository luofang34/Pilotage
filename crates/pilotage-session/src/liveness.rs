//! Holder-liveness bookkeeping for the session engine's frame-silence watchdog
//! (ADR-0006, ADR-0008, ADR-0010).
//!
//! For each scope with an effective holder, the engine records the deadline by
//! which that holder must send a fresh, accepted control frame. A holder that
//! goes silent past its deadline — while its WebTransport connection is still
//! open, so no disconnect ever fires — is judged link-lost and released. This
//! module only tracks deadlines; the engine issues the release and the
//! neutralization, keeping every clock read explicit.
//!
//! The tracker is allocation-free after configuration: slots are
//! pre-allocated for every registered scope (an entry can only exist for a
//! registered scope, so the capacity is exact), the per-frame refresh
//! updates a slot in place through borrowed keys, and expiry moves slots
//! into a caller-provided reusable buffer.

use pilotage_protocol::{PrincipalId, ScopeId, VehicleId};
use pilotage_timing::MonoTimestamp;

use crate::clients::ScopePair;

/// One scope with an effective holder and the instant by which that holder
/// must send another accepted frame to stay live.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeldSlot {
    vehicle: VehicleId,
    scope: ScopeId,
    principal: PrincipalId,
    deadline: MonoTimestamp,
}

/// A holder whose frame-silence deadline has passed, moved out of the
/// tracker for the engine to release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExpiredHolder {
    /// Vehicle of the silent scope.
    pub(crate) vehicle: VehicleId,
    /// The silent scope.
    pub(crate) scope: ScopeId,
    /// The holder that went silent.
    pub(crate) principal: PrincipalId,
}

/// Per-scope frame-silence deadlines for every scope that has a holder.
///
/// An entry exists exactly while a scope has an effective holder: the engine
/// marks it active on grant/handover/override, refreshes it on accepted
/// axis-bearing frames, and clears it when the holder is released. No entry
/// means no holder to watch.
#[derive(Debug, Default)]
pub(crate) struct HolderLiveness {
    entries: Vec<HeldSlot>,
}

impl HolderLiveness {
    /// Creates a tracker with capacity for `scope_count` watched scopes —
    /// the number of registered scopes, so the entry vector never grows
    /// after construction.
    pub(crate) fn with_scope_capacity(scope_count: usize) -> Self {
        Self {
            entries: Vec::with_capacity(scope_count),
        }
    }

    /// Records that `principal` holds `pair` and must send another accepted
    /// frame before `deadline`. Replaces any prior entry for the scope, so a
    /// handover to a new holder resets the clock under the new principal.
    pub(crate) fn mark_active(
        &mut self,
        pair: ScopePair,
        principal: PrincipalId,
        deadline: MonoTimestamp,
    ) {
        let (vehicle, scope) = pair;
        if let Some(slot) = self.slot_mut(vehicle, &scope) {
            slot.principal = principal;
            slot.deadline = deadline;
            return;
        }
        self.entries.push(HeldSlot {
            vehicle,
            scope,
            principal,
            deadline,
        });
    }

    /// Pushes the deadline of an existing entry forward, in place and
    /// without cloning the scope key — the per-accepted-frame hot path.
    /// Only the recorded holder refreshes its own deadline; an entry is
    /// never created here, so a frame cannot resurrect a released scope.
    pub(crate) fn refresh(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        principal: PrincipalId,
        deadline: MonoTimestamp,
    ) {
        if let Some(slot) = self.slot_mut(vehicle, scope)
            && slot.principal == principal
        {
            slot.deadline = deadline;
        }
    }

    /// Stops watching `pair` (its holder was released). Idempotent.
    pub(crate) fn clear(&mut self, pair: &ScopePair) {
        let (vehicle, scope) = pair;
        if let Some(index) = self
            .entries
            .iter()
            .position(|slot| slot.vehicle == *vehicle && slot.scope == *scope)
        {
            self.entries.swap_remove(index);
        }
    }

    /// The earliest holder-silence deadline across all watched scopes, if any.
    /// The engine folds this into its next-tick deadline so a tick lands when a
    /// holder would time out.
    pub(crate) fn next_deadline(&self) -> Option<MonoTimestamp> {
        self.entries.iter().map(|slot| slot.deadline).min()
    }

    /// Moves every entry whose deadline is at or before `now` — the holders
    /// that have gone silent and must be released — into `out`, sorted by
    /// `(vehicle, scope)` for deterministic effect ordering. `out` is the
    /// engine's reusable scratch buffer, so the steady state allocates
    /// nothing.
    pub(crate) fn expire_into(&mut self, now: MonoTimestamp, out: &mut Vec<ExpiredHolder>) {
        let mut index = 0;
        while index < self.entries.len() {
            if self.entries[index].deadline <= now {
                let slot = self.entries.swap_remove(index);
                out.push(ExpiredHolder {
                    vehicle: slot.vehicle,
                    scope: slot.scope,
                    principal: slot.principal,
                });
            } else {
                index = index.saturating_add(1);
            }
        }
        out.sort_by(|a, b| (a.vehicle, &a.scope).cmp(&(b.vehicle, &b.scope)));
    }

    fn slot_mut(&mut self, vehicle: VehicleId, scope: &ScopeId) -> Option<&mut HeldSlot> {
        self.entries
            .iter_mut()
            .find(|slot| slot.vehicle == vehicle && slot.scope == *scope)
    }
}

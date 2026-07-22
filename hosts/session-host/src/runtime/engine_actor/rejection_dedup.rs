//! Transition deduplication for adapter-boundary frame rejections.

use std::collections::BTreeMap;

use pilotage_protocol::{FrameRejectionReason, Generation, ScopeId, VehicleId};
use pilotage_session::ClientKey;

type RejectionKey = (ClientKey, VehicleId, ScopeId);
type RejectionState = (Generation, FrameRejectionReason);

/// One reliable notice per adapter rejection transition and authority epoch.
#[derive(Debug, Default)]
pub(super) struct RejectionDedup {
    active: BTreeMap<RejectionKey, RejectionState>,
}

impl RejectionDedup {
    /// Records a rejection and reports whether its transition needs a notice.
    pub(super) fn should_notify(
        &mut self,
        client: ClientKey,
        vehicle: VehicleId,
        scope: &ScopeId,
        generation: Generation,
        reason: FrameRejectionReason,
    ) -> bool {
        let key = (client, vehicle, scope.clone());
        let state = (generation, reason);
        if self.active.get(&key) == Some(&state) {
            return false;
        }
        self.active.insert(key, state);
        true
    }

    /// An enacted frame ends the active rejection transition for this scope.
    pub(super) fn observe_enacted(
        &mut self,
        client: ClientKey,
        vehicle: VehicleId,
        scope: &ScopeId,
    ) {
        self.active.remove(&(client, vehicle, scope.clone()));
    }

    /// Drops every transition owned by a retired connection.
    pub(super) fn forget(&mut self, client: ClientKey) {
        self.active.retain(|(owner, _, _), _| *owner != client);
    }
}

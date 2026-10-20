//! Catalog replacements preserve session identity but never grant authority.
use super::*;

impl ClientEngine {
    pub(super) fn on_capabilities(
        &mut self,
        capabilities: wire::HostCapabilities,
    ) -> Vec<ClientAction> {
        let Some(previous) = self.admission.as_ref() else {
            return Vec::new();
        };
        let welcome = wire::ServerWelcome {
            session: Some(wire::SessionId {
                value: previous.session_id,
            }),
            principal: Some(wire::PrincipalId {
                value: previous.principal_id,
            }),
            host_capabilities: Some(capabilities),
            scope_holders: previous.scope_holders.clone(),
        };
        let Some(next) = Admission::from_welcome(&welcome) else {
            return Vec::new();
        };
        self.lanes.retain(|(vehicle, scope), _| {
            let old = find_scope(previous, *vehicle, scope);
            let new = find_scope(&next, *vehicle, scope);
            old.is_some() && old == new
        });
        self.pending_leases.retain(|(vehicle, scope), _| {
            find_scope(previous, *vehicle, scope).is_some()
                && find_scope(previous, *vehicle, scope) == find_scope(&next, *vehicle, scope)
        });
        if self
            .pending_takeover
            .as_ref()
            .is_some_and(|(vehicle, scope)| {
                find_scope(previous, *vehicle, scope) != find_scope(&next, *vehicle, scope)
            })
        {
            self.pending_takeover = None;
        }
        self.admission = Some(next.clone());
        vec![ClientAction::Emit(ModuleEvent::CapabilitiesChanged(next))]
    }
}

fn find_scope<'a>(
    admission: &'a Admission,
    vehicle: u64,
    scope: &str,
) -> Option<&'a crate::ScopeCatalog> {
    admission
        .vehicles
        .iter()
        .find(|value| value.vehicle_id == vehicle)?
        .scopes
        .iter()
        .find(|value| value.scope == scope)
}

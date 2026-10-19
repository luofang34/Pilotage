//! Reliable capability refresh with authority fencing at the contract boundary.
use super::{Actions, SessionEngine};
use crate::{LinkLossTrigger, OutboundMessage, SessionAction, SessionOutcome};
use pilotage_adapter_api::AdapterCapabilities;
use pilotage_authority::{AuthorityClass, AuthorityCommand};
use pilotage_protocol::ScopeId;
use pilotage_timing::MonoTimestamp;
use std::collections::HashSet;

impl SessionEngine {
    pub(super) fn invalidate_pending_for(
        &self,
        effect: &pilotage_authority::AuthorityEffect,
        actions: &mut Actions,
    ) {
        use pilotage_authority::AuthorityEffect as Effect;
        match effect {
            Effect::ScopeLeaseRevoked { vehicle, scope, .. }
            | Effect::HolderLinkLost { vehicle, scope, .. }
            | Effect::ScopeTransferCommitted { vehicle, scope, .. }
            | Effect::EmergencyOverrideApplied { vehicle, scope, .. } => {
                actions.push_safety(SessionAction::InvalidatePendingCommands {
                    vehicle: *vehicle,
                    scope: scope.clone(),
                });
            }
            _ => {}
        }
    }
    /// Replace the live capability catalog; changed held contracts lose authority first.
    #[must_use]
    pub fn refresh_capabilities(
        &mut self,
        capabilities: AdapterCapabilities,
        now: MonoTimestamp,
    ) -> SessionOutcome {
        let mut actions = Actions::new(self.config.max_actions_per_call);
        if self.capabilities == capabilities {
            return actions.into_outcome();
        }
        let changed = self.changed_groups(&capabilities);
        for (vehicle, scope) in changed {
            let effects = self.authority.handle(
                AuthorityCommand::Revoke {
                    vehicle,
                    scope,
                    authority_class: AuthorityClass::Administrator,
                },
                now,
            );
            self.fan_out_authority(
                effects,
                now,
                LinkLossTrigger::AuthorityRevoked,
                &mut actions,
            );
        }
        self.register_catalog(&capabilities, now);
        self.link_loss
            .refresh_policies(&capabilities, &self.config.link_loss_overrides);
        self.capabilities = capabilities;
        actions.push_safety(SessionAction::Broadcast {
            envelope: OutboundMessage::Capabilities(crate::capabilities::host_capabilities(
                &self.capabilities,
                &self.config.host_version,
            )),
        });
        actions.into_outcome()
    }

    fn changed_groups(&self, next: &AdapterCapabilities) -> HashSet<crate::clients::ScopePair> {
        self.capabilities
            .vehicles
            .iter()
            .flat_map(|vehicle| {
                vehicle.scopes.iter().filter_map(|scope| {
                    let replacement =
                        crate::capabilities::scope_capability(next, vehicle.id, &scope.scope);
                    (replacement != Some(scope))
                        .then(|| self.authority_pair(vehicle.id, &scope.scope))
                        .flatten()
                })
            })
            .collect()
    }

    fn register_catalog(&mut self, next: &AdapterCapabilities, now: MonoTimestamp) {
        self.groups.clear();
        self.command_only.clear();
        let mut continuous = HashSet::new();
        for vehicle in &next.vehicles {
            for descriptor in &vehicle.scopes {
                let group = descriptor.authority_group.as_ref().map_or_else(
                    || descriptor.scope.clone(),
                    |name| ScopeId::new(name.clone()),
                );
                let pair = (vehicle.id, group.clone());
                self.groups
                    .insert((vehicle.id, descriptor.scope.clone()), group.clone());
                let streams = !descriptor.intents.is_empty()
                    || descriptor.legacy.is_some()
                    || !descriptor.axes.is_empty();
                if streams {
                    continuous.insert(pair.clone());
                } else {
                    self.command_only.insert(pair.clone());
                }
                if !self.clients.is_registered(&pair) {
                    self.clients.register_scope(pair);
                    drop(self.authority.handle(
                        AuthorityCommand::RegisterScope {
                            vehicle: vehicle.id,
                            scope: group,
                        },
                        now,
                    ));
                }
            }
        }
        self.command_only.retain(|pair| !continuous.contains(pair));
    }
}

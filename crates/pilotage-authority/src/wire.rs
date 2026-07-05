//! Conversion from engine effects to protocol wire authority events
//! (ADR-0012, ADR-0014).
//!
//! Each [`AuthorityEffect`] classifies into the stable ADR-0012 authority
//! event vocabulary via [`WireEventKind`], and converts into a
//! `pilotage.v1.AuthorityEvent` via `From<&AuthorityEffect>`. Both mappings
//! are total over the effect enum: authority effects are the persisted audit
//! trail for handover and override disputes (ADR-0012), so a conversion that
//! skipped a variant would silently drop an audit event.
//!
//! The wire vocabulary is intentionally coarser than the effect enum:
//! several effects share one wire message (a release and a revoke both
//! serialize to `ScopeLeaseRevoked`; rejections and holder link loss
//! serialize to `WarningRaised`).

mod warnings;

use pilotage_protocol::wire as proto;
use pilotage_protocol::wire::authority_event::Event;
use pilotage_protocol::{Generation, PrincipalId, ScopeId, VehicleId};

use crate::command::{AuthorityClass, LinkState, OverrideReason};
use crate::effect::AuthorityEffect;
use warnings::{confirmation_warning, holder_link_lost, rejected};

/// The ADR-0012 authority event class a given effect serializes to.
///
/// This is the wire-facing classification of an [`AuthorityEffect`]. It is
/// intentionally coarser than the effect enum: several internal effects share
/// one wire event name (for example both a release and a revoke serialize to
/// `ScopeLeaseRevoked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WireEventKind {
    /// A scope became known (`ScopeRegistered`).
    ScopeRegistered,
    /// A lease was granted (`ScopeLeaseGranted`).
    ScopeLeaseGranted,
    /// A transfer was offered (`ScopeTransferOffered`).
    ScopeTransferOffered,
    /// A transfer was committed (`ScopeTransferCommitted`).
    ScopeTransferCommitted,
    /// A pending offer expired (`ScopeTransferExpired`).
    ScopeTransferExpired,
    /// A lease was revoked or released (`ScopeLeaseRevoked`).
    ScopeLeaseRevoked,
    /// An emergency override was applied (`EmergencyOverrideApplied`).
    EmergencyOverrideApplied,
    /// A repeat override was already effective (`EmergencyOverrideApplied`
    /// with the `already_effective` idempotency flag on the wire).
    EmergencyOverrideAlreadyEffective,
    /// A holder link state changed (`LinkStateChanged`).
    LinkStateChanged,
    /// The effective holder's link was lost and the scope released
    /// (`WarningRaised` on the wire; the engine emits the accompanying link
    /// transition as its own [`AuthorityEffect::LinkStateChanged`] effect,
    /// so the wire sequence is `LinkStateChanged` + `WarningRaised`).
    HolderLinkLost,
    /// A non-fatal warning was raised (`WarningRaised`).
    WarningRaised,
    /// A command was rejected (`WarningRaised` on the wire; no lease event).
    CommandRejected,
}

impl AuthorityEffect {
    /// Classifies this effect into its ADR-0012 wire event kind.
    ///
    /// Total over every effect variant: the audit trail cannot drop an event.
    #[must_use]
    pub fn wire_event_kind(&self) -> WireEventKind {
        match self {
            AuthorityEffect::ScopeRegistered { .. } => WireEventKind::ScopeRegistered,
            AuthorityEffect::ScopeLeaseGranted { .. } => WireEventKind::ScopeLeaseGranted,
            AuthorityEffect::ScopeTransferOffered { .. } => WireEventKind::ScopeTransferOffered,
            AuthorityEffect::ScopeTransferCommitted { .. } => WireEventKind::ScopeTransferCommitted,
            AuthorityEffect::ScopeTransferExpired { .. } => WireEventKind::ScopeTransferExpired,
            AuthorityEffect::ScopeLeaseRevoked { .. } => WireEventKind::ScopeLeaseRevoked,
            AuthorityEffect::EmergencyOverrideApplied { .. } => {
                WireEventKind::EmergencyOverrideApplied
            }
            AuthorityEffect::EmergencyOverrideAlreadyEffective { .. } => {
                WireEventKind::EmergencyOverrideAlreadyEffective
            }
            AuthorityEffect::LinkStateChanged { .. } => WireEventKind::LinkStateChanged,
            AuthorityEffect::HolderLinkLost { .. } => WireEventKind::HolderLinkLost,
            AuthorityEffect::WarningRaised { .. } => WireEventKind::WarningRaised,
            AuthorityEffect::CommandRejected { .. } => WireEventKind::CommandRejected,
        }
    }
}

impl From<&AuthorityEffect> for proto::AuthorityEvent {
    /// Builds the wire authority event this effect serializes to, carrying
    /// the effect's identifiers, generation, holder, authority class, and
    /// override reason.
    ///
    /// Total over every effect variant, matching
    /// [`AuthorityEffect::wire_event_kind`]: the audit trail cannot drop an
    /// event.
    fn from(effect: &AuthorityEffect) -> Self {
        use AuthorityEffect as E;
        let event = match effect {
            E::ScopeRegistered { vehicle, scope } => registered(*vehicle, scope),
            E::ScopeLeaseGranted {
                vehicle,
                scope,
                holder,
                generation,
            } => granted(*vehicle, scope, *holder, *generation),
            E::ScopeTransferOffered {
                vehicle,
                scope,
                from,
                to,
                generation,
                expires_at: _,
            } => offered(*vehicle, scope, *from, *to, *generation),
            E::ScopeTransferCommitted {
                vehicle,
                scope,
                from,
                to,
                generation,
            } => committed(*vehicle, scope, *from, *to, *generation),
            E::ScopeTransferExpired {
                vehicle,
                scope,
                holder,
                generation,
            } => expired(*vehicle, scope, *holder, *generation),
            E::ScopeLeaseRevoked {
                vehicle,
                scope,
                previous_holder,
                generation,
            } => revoked(*vehicle, scope, *previous_holder, *generation),
            E::EmergencyOverrideApplied {
                vehicle,
                scope,
                previous_holder,
                holder,
                authority_class,
                reason,
                generation,
            } => override_applied(
                *vehicle,
                scope,
                *previous_holder,
                *holder,
                *authority_class,
                reason,
                *generation,
            ),
            E::EmergencyOverrideAlreadyEffective {
                vehicle,
                scope,
                holder,
                authority_class,
                generation,
            } => {
                override_already_effective(*vehicle, scope, *holder, *authority_class, *generation)
            }
            E::LinkStateChanged {
                vehicle,
                scope,
                principal,
                state,
            } => link_changed(*vehicle, scope, *principal, *state),
            E::HolderLinkLost {
                vehicle,
                scope,
                lost_holder,
                generation,
            } => holder_link_lost(*vehicle, scope, *lost_holder, *generation),
            E::WarningRaised {
                vehicle,
                scope,
                warning: raised,
            } => confirmation_warning(*vehicle, scope, raised),
            E::CommandRejected {
                vehicle,
                scope,
                reason,
            } => rejected(*vehicle, scope, reason),
        };
        proto::AuthorityEvent { event: Some(event) }
    }
}

fn wire_principal(principal: PrincipalId) -> proto::PrincipalId {
    proto::PrincipalId {
        value: principal.as_u64(),
    }
}

fn wire_vehicle(vehicle: VehicleId) -> proto::VehicleId {
    proto::VehicleId {
        value: vehicle.as_u64(),
    }
}

fn wire_scope(scope: &ScopeId) -> proto::ScopeId {
    proto::ScopeId {
        value: scope.as_str().to_owned(),
    }
}

fn wire_generation(generation: Generation) -> proto::Generation {
    proto::Generation {
        value: generation.as_u64(),
    }
}

fn wire_class(class: AuthorityClass) -> i32 {
    let class = match class {
        AuthorityClass::Operator => proto::AuthorityClass::Operator,
        AuthorityClass::Supervisor => proto::AuthorityClass::Supervisor,
        AuthorityClass::Administrator => proto::AuthorityClass::Administrator,
        AuthorityClass::Automation => proto::AuthorityClass::Automation,
    };
    class as i32
}

fn wire_link(state: LinkState) -> i32 {
    let state = match state {
        LinkState::Nominal => proto::LinkState::Nominal,
        LinkState::Degraded => proto::LinkState::Degraded,
        LinkState::Lost => proto::LinkState::Lost,
    };
    state as i32
}

fn registered(vehicle: VehicleId, scope: &ScopeId) -> Event {
    Event::ScopeRegistered(proto::ScopeRegistered {
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
    })
}

fn granted(
    vehicle: VehicleId,
    scope: &ScopeId,
    holder: PrincipalId,
    generation: Generation,
) -> Event {
    Event::ScopeLeaseGranted(proto::ScopeLeaseGranted {
        principal: Some(wire_principal(holder)),
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        generation: Some(wire_generation(generation)),
        reason: String::new(),
        authority_class: proto::AuthorityClass::Unspecified as i32,
    })
}

fn offered(
    vehicle: VehicleId,
    scope: &ScopeId,
    from: PrincipalId,
    to: PrincipalId,
    generation: Generation,
) -> Event {
    Event::ScopeTransferOffered(proto::ScopeTransferOffered {
        from_principal: Some(wire_principal(from)),
        to_principal: Some(wire_principal(to)),
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        generation: Some(wire_generation(generation)),
        reason: String::new(),
        authority_class: proto::AuthorityClass::Unspecified as i32,
    })
}

fn committed(
    vehicle: VehicleId,
    scope: &ScopeId,
    from: PrincipalId,
    to: PrincipalId,
    generation: Generation,
) -> Event {
    Event::ScopeTransferCommitted(proto::ScopeTransferCommitted {
        from_principal: Some(wire_principal(from)),
        to_principal: Some(wire_principal(to)),
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        generation: Some(wire_generation(generation)),
        reason: String::new(),
        authority_class: proto::AuthorityClass::Unspecified as i32,
    })
}

fn expired(
    vehicle: VehicleId,
    scope: &ScopeId,
    holder: PrincipalId,
    generation: Generation,
) -> Event {
    Event::ScopeTransferExpired(proto::ScopeTransferExpired {
        holder: Some(wire_principal(holder)),
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        generation: Some(wire_generation(generation)),
    })
}

fn revoked(
    vehicle: VehicleId,
    scope: &ScopeId,
    previous_holder: Option<PrincipalId>,
    generation: Generation,
) -> Event {
    Event::ScopeLeaseRevoked(proto::ScopeLeaseRevoked {
        principal: previous_holder.map(wire_principal),
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        generation: Some(wire_generation(generation)),
        reason: String::new(),
        authority_class: proto::AuthorityClass::Unspecified as i32,
    })
}

fn override_applied(
    vehicle: VehicleId,
    scope: &ScopeId,
    previous_holder: Option<PrincipalId>,
    holder: PrincipalId,
    class: AuthorityClass,
    reason: &OverrideReason,
    generation: Generation,
) -> Event {
    Event::EmergencyOverrideApplied(proto::EmergencyOverrideApplied {
        principal: Some(wire_principal(holder)),
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        generation: Some(wire_generation(generation)),
        reason: reason.as_str().to_owned(),
        authority_class: wire_class(class),
        previous_holder: previous_holder.map(wire_principal),
        already_effective: false,
    })
}

fn override_already_effective(
    vehicle: VehicleId,
    scope: &ScopeId,
    holder: PrincipalId,
    class: AuthorityClass,
    generation: Generation,
) -> Event {
    Event::EmergencyOverrideApplied(proto::EmergencyOverrideApplied {
        principal: Some(wire_principal(holder)),
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        generation: Some(wire_generation(generation)),
        reason: String::new(),
        authority_class: wire_class(class),
        previous_holder: None,
        already_effective: true,
    })
}

fn link_changed(
    vehicle: VehicleId,
    scope: &ScopeId,
    principal: PrincipalId,
    state: LinkState,
) -> Event {
    Event::LinkStateChanged(proto::LinkStateChanged {
        principal: Some(wire_principal(principal)),
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        state: wire_link(state),
    })
}

#[cfg(test)]
mod tests;

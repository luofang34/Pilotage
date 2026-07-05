//! Serialization of warning-class effects into the ADR-0012 `WarningRaised`
//! wire event.
//!
//! Three effect families collapse onto `WarningRaised`: out-of-order
//! confirmations, typed command rejections, and the link-loss release. The
//! `WarningKind` field keeps them machine-distinguishable on the wire.

use pilotage_protocol::wire as proto;
use pilotage_protocol::wire::authority_event::Event;
use pilotage_protocol::{Generation, PrincipalId, ScopeId, VehicleId};

use super::{wire_generation, wire_principal, wire_scope, wire_vehicle};
use crate::effect::{AuthorityWarning, RejectReason};

/// Serializes the link-loss release as a `WarningRaised` wire event; the
/// engine emits the accompanying link transition as its own
/// [`LinkStateChanged`] effect.
///
/// [`LinkStateChanged`]: crate::AuthorityEffect::LinkStateChanged
pub(super) fn holder_link_lost(
    vehicle: VehicleId,
    scope: &ScopeId,
    lost_holder: PrincipalId,
    generation: Generation,
) -> Event {
    warning(
        vehicle,
        scope,
        proto::WarningKind::HolderLinkLost,
        Some(lost_holder),
        None,
        Some(generation),
        "effective holder link lost; scope released".to_owned(),
    )
}

fn warning(
    vehicle: VehicleId,
    scope: &ScopeId,
    kind: proto::WarningKind,
    principal: Option<PrincipalId>,
    current_holder: Option<PrincipalId>,
    generation: Option<Generation>,
    detail: String,
) -> Event {
    Event::WarningRaised(proto::WarningRaised {
        vehicle: Some(wire_vehicle(vehicle)),
        scope: Some(wire_scope(scope)),
        kind: kind as i32,
        principal: principal.map(wire_principal),
        current_holder: current_holder.map(wire_principal),
        generation: generation.map(wire_generation),
        detail,
    })
}

/// Serializes an out-of-order confirmation warning (ADR-0012
/// `WarningRaised`).
pub(super) fn confirmation_warning(
    vehicle: VehicleId,
    scope: &ScopeId,
    raised: &AuthorityWarning,
) -> Event {
    match raised {
        AuthorityWarning::UnexpectedIHave { by, current_holder } => warning(
            vehicle,
            scope,
            proto::WarningKind::UnexpectedIHave,
            Some(*by),
            *current_holder,
            None,
            "\"I have control\" confirmation from a principal that is not the effective holder"
                .to_owned(),
        ),
        AuthorityWarning::UnexpectedYouHave { by, current_holder } => warning(
            vehicle,
            scope,
            proto::WarningKind::UnexpectedYouHave,
            Some(*by),
            *current_holder,
            None,
            "\"you have control\" confirmation from a principal that is not the effective holder"
                .to_owned(),
        ),
    }
}

/// Serializes a typed command rejection as a `WarningRaised` wire event.
///
/// The reason's context flows into the structured `principal`,
/// `current_holder`, and `generation` fields where one of those fields fits
/// it, and into `detail` otherwise, so the wire event never carries less
/// context than the typed [`RejectReason`].
pub(super) fn rejected(vehicle: VehicleId, scope: &ScopeId, reason: &RejectReason) -> Event {
    let kind = proto::WarningKind::CommandRejected;
    let simple = |detail: &str| warning(vehicle, scope, kind, None, None, None, detail.to_owned());
    match reason {
        RejectReason::UnknownScope => simple("command targeted an unregistered scope"),
        RejectReason::ScopeAlreadyRegistered => simple("scope is already registered"),
        RejectReason::ScopeUnassigned => {
            simple("command requires a held scope but it is unassigned")
        }
        RejectReason::OfferAlreadyPending => simple("scope already has a pending transfer offer"),
        RejectReason::NoPendingOffer => simple("accept targeted a scope with no pending offer"),
        RejectReason::ScopeNotUnassigned { current_holder } => warning(
            vehicle,
            scope,
            kind,
            None,
            Some(*current_holder),
            None,
            "grant targeted a scope that is not unassigned".to_owned(),
        ),
        RejectReason::NotCurrentHolder {
            actor,
            current_holder,
        } => warning(
            vehicle,
            scope,
            kind,
            Some(*actor),
            *current_holder,
            None,
            "command actor is not the effective holder".to_owned(),
        ),
        RejectReason::NotOfferRecipient { actor, expected } => warning(
            vehicle,
            scope,
            kind,
            Some(*actor),
            None,
            None,
            format!(
                "accept from a principal other than the offer recipient (expected principal {})",
                expected.as_u64()
            ),
        ),
        RejectReason::GenerationMismatch { supplied, current } => warning(
            vehicle,
            scope,
            kind,
            None,
            None,
            Some(*current),
            format!(
                "stale accept fenced out: supplied generation {}, current generation {}",
                supplied.as_u64(),
                current.as_u64()
            ),
        ),
    }
}

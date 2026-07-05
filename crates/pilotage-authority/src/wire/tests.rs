//! Unit tests for effect-to-wire authority-event conversion.
//!
//! Coverage here is the guardrail for the ADR-0012 totality requirement:
//! every [`AuthorityEffect`] variant must produce a wire event whose oneof
//! arm agrees with its [`WireEventKind`], and every produced event must
//! survive an encode/decode round trip through the envelope.

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashSet;

use pilotage_protocol::wire as proto;
use pilotage_protocol::wire::authority_event::Event;
use pilotage_protocol::{
    Generation, PrincipalId, SCHEMA_VERSION, ScopeId, VehicleId, decode_envelope_length_delimited,
    encode_envelope_length_delimited,
};
use pilotage_timing::MonoTimestamp;

use super::WireEventKind;
use crate::command::{AuthorityClass, LinkState, OverrideReason};
use crate::effect::{AuthorityEffect, AuthorityWarning, RejectReason};

fn vehicle() -> VehicleId {
    VehicleId::new(7)
}

fn scope() -> ScopeId {
    ScopeId::new("vehicle.motion")
}

fn p(value: u64) -> PrincipalId {
    PrincipalId::new(value)
}

fn g(value: u64) -> Generation {
    Generation::new(value)
}

/// The lease- and transfer-shaped half of the per-variant samples.
fn lease_effect_samples() -> Vec<AuthorityEffect> {
    vec![
        AuthorityEffect::ScopeRegistered {
            vehicle: vehicle(),
            scope: scope(),
        },
        AuthorityEffect::ScopeLeaseGranted {
            vehicle: vehicle(),
            scope: scope(),
            holder: p(1),
            generation: g(1),
        },
        AuthorityEffect::ScopeTransferOffered {
            vehicle: vehicle(),
            scope: scope(),
            from: p(1),
            to: p(2),
            generation: g(1),
            expires_at: MonoTimestamp::from_nanos(50),
        },
        AuthorityEffect::ScopeTransferCommitted {
            vehicle: vehicle(),
            scope: scope(),
            from: p(1),
            to: p(2),
            generation: g(2),
        },
        AuthorityEffect::ScopeTransferExpired {
            vehicle: vehicle(),
            scope: scope(),
            holder: p(1),
            generation: g(1),
        },
        AuthorityEffect::ScopeLeaseRevoked {
            vehicle: vehicle(),
            scope: scope(),
            previous_holder: Some(p(1)),
            generation: g(2),
        },
    ]
}

/// The override-, link-, and warning-shaped half of the per-variant samples.
fn signal_effect_samples() -> Vec<AuthorityEffect> {
    vec![
        AuthorityEffect::EmergencyOverrideApplied {
            vehicle: vehicle(),
            scope: scope(),
            previous_holder: Some(p(1)),
            holder: p(3),
            authority_class: AuthorityClass::Supervisor,
            reason: OverrideReason::new("runaway vehicle"),
            generation: g(3),
        },
        AuthorityEffect::EmergencyOverrideAlreadyEffective {
            vehicle: vehicle(),
            scope: scope(),
            holder: p(3),
            authority_class: AuthorityClass::Supervisor,
            generation: g(3),
        },
        AuthorityEffect::LinkStateChanged {
            vehicle: vehicle(),
            scope: scope(),
            principal: p(1),
            state: LinkState::Degraded,
        },
        AuthorityEffect::HolderLinkLost {
            vehicle: vehicle(),
            scope: scope(),
            lost_holder: p(1),
            generation: g(4),
        },
        AuthorityEffect::WarningRaised {
            vehicle: vehicle(),
            scope: scope(),
            warning: AuthorityWarning::UnexpectedIHave {
                by: p(2),
                current_holder: Some(p(1)),
            },
        },
        AuthorityEffect::CommandRejected {
            vehicle: vehicle(),
            scope: scope(),
            reason: RejectReason::GenerationMismatch {
                supplied: g(1),
                current: g(2),
            },
        },
    ]
}

/// One sample effect per [`AuthorityEffect`] variant.
///
/// A new effect variant breaks the `From` conversion at compile time until
/// it is mapped; `samples_cover_every_wire_event_kind` then fails until a
/// sample for the new variant is added here.
fn sample_effects() -> Vec<AuthorityEffect> {
    let mut samples = lease_effect_samples();
    samples.extend(signal_effect_samples());
    samples
}

/// One sample per [`RejectReason`] variant.
fn all_reject_reasons() -> Vec<RejectReason> {
    vec![
        RejectReason::UnknownScope,
        RejectReason::ScopeAlreadyRegistered,
        RejectReason::ScopeNotUnassigned {
            current_holder: p(1),
        },
        RejectReason::NotCurrentHolder {
            actor: p(2),
            current_holder: Some(p(1)),
        },
        RejectReason::ScopeUnassigned,
        RejectReason::OfferAlreadyPending,
        RejectReason::NoPendingOffer,
        RejectReason::NotOfferRecipient {
            actor: p(2),
            expected: p(3),
        },
        RejectReason::GenerationMismatch {
            supplied: g(1),
            current: g(2),
        },
    ]
}

/// Whether `event` is the oneof arm that `kind` documents it serializes to.
fn arm_matches(kind: WireEventKind, event: &Event) -> bool {
    matches!(
        (kind, event),
        (WireEventKind::ScopeRegistered, Event::ScopeRegistered(_))
            | (
                WireEventKind::ScopeLeaseGranted,
                Event::ScopeLeaseGranted(_)
            )
            | (
                WireEventKind::ScopeTransferOffered,
                Event::ScopeTransferOffered(_)
            )
            | (
                WireEventKind::ScopeTransferCommitted,
                Event::ScopeTransferCommitted(_)
            )
            | (
                WireEventKind::ScopeTransferExpired,
                Event::ScopeTransferExpired(_)
            )
            | (
                WireEventKind::ScopeLeaseRevoked,
                Event::ScopeLeaseRevoked(_)
            )
            | (
                WireEventKind::EmergencyOverrideApplied,
                Event::EmergencyOverrideApplied(_)
            )
            | (
                WireEventKind::EmergencyOverrideAlreadyEffective,
                Event::EmergencyOverrideApplied(_)
            )
            | (WireEventKind::LinkStateChanged, Event::LinkStateChanged(_))
            | (WireEventKind::HolderLinkLost, Event::WarningRaised(_))
            | (WireEventKind::WarningRaised, Event::WarningRaised(_))
            | (WireEventKind::CommandRejected, Event::WarningRaised(_))
    )
}

/// Converts an effect and unwraps the oneof arm every effect must produce.
fn to_event(effect: &AuthorityEffect) -> Event {
    proto::AuthorityEvent::from(effect)
        .event
        .expect("every effect must map to a wire event")
}

#[test]
fn every_effect_variant_produces_its_wire_event() {
    for effect in sample_effects() {
        let event = to_event(&effect);
        assert!(
            arm_matches(effect.wire_event_kind(), &event),
            "wrong oneof arm for {effect:?}: {event:?}"
        );
    }
}

#[test]
fn samples_cover_every_wire_event_kind() {
    let covered: HashSet<_> = sample_effects()
        .iter()
        .map(AuthorityEffect::wire_event_kind)
        .collect();
    let expected: HashSet<_> = [
        WireEventKind::ScopeRegistered,
        WireEventKind::ScopeLeaseGranted,
        WireEventKind::ScopeTransferOffered,
        WireEventKind::ScopeTransferCommitted,
        WireEventKind::ScopeTransferExpired,
        WireEventKind::ScopeLeaseRevoked,
        WireEventKind::EmergencyOverrideApplied,
        WireEventKind::EmergencyOverrideAlreadyEffective,
        WireEventKind::LinkStateChanged,
        WireEventKind::HolderLinkLost,
        WireEventKind::WarningRaised,
        WireEventKind::CommandRejected,
    ]
    .into_iter()
    .collect();
    assert_eq!(covered, expected);
}

#[test]
fn every_wire_event_round_trips_through_an_envelope() {
    for effect in sample_effects() {
        let event = proto::AuthorityEvent::from(&effect);
        let envelope = proto::Envelope {
            schema_version: SCHEMA_VERSION,
            payload: Some(proto::envelope::Payload::AuthorityEvent(event)),
        };
        let bytes = encode_envelope_length_delimited(&envelope);
        let (decoded, rest) =
            decode_envelope_length_delimited(&bytes).expect("envelope must decode");
        assert!(rest.is_empty());
        assert_eq!(decoded, envelope, "round-trip mismatch for {effect:?}");
    }
}

#[test]
fn override_event_carries_holder_class_reason_and_generation() {
    let effect = AuthorityEffect::EmergencyOverrideApplied {
        vehicle: vehicle(),
        scope: scope(),
        previous_holder: Some(p(1)),
        holder: p(3),
        authority_class: AuthorityClass::Supervisor,
        reason: OverrideReason::new("runaway vehicle"),
        generation: g(3),
    };
    let Event::EmergencyOverrideApplied(msg) = to_event(&effect) else {
        panic!("expected the override arm");
    };
    assert_eq!(msg.principal, Some(proto::PrincipalId { value: 3 }));
    assert_eq!(msg.previous_holder, Some(proto::PrincipalId { value: 1 }));
    assert_eq!(msg.vehicle, Some(proto::VehicleId { value: 7 }));
    assert_eq!(
        msg.scope,
        Some(proto::ScopeId {
            value: "vehicle.motion".to_owned()
        })
    );
    assert_eq!(msg.generation, Some(proto::Generation { value: 3 }));
    assert_eq!(
        msg.authority_class,
        proto::AuthorityClass::Supervisor as i32
    );
    assert_eq!(msg.reason, "runaway vehicle");
    assert!(!msg.already_effective);
}

#[test]
fn repeat_override_sets_the_idempotency_flag() {
    let effect = AuthorityEffect::EmergencyOverrideAlreadyEffective {
        vehicle: vehicle(),
        scope: scope(),
        holder: p(3),
        authority_class: AuthorityClass::Supervisor,
        generation: g(3),
    };
    let Event::EmergencyOverrideApplied(msg) = to_event(&effect) else {
        panic!("expected the override arm");
    };
    assert!(msg.already_effective);
    assert_eq!(msg.principal, Some(proto::PrincipalId { value: 3 }));
    assert_eq!(msg.previous_holder, None);
    assert_eq!(msg.generation, Some(proto::Generation { value: 3 }));
    assert_eq!(
        msg.authority_class,
        proto::AuthorityClass::Supervisor as i32
    );
}

#[test]
fn link_state_change_carries_the_wire_link_state() {
    let effect = AuthorityEffect::LinkStateChanged {
        vehicle: vehicle(),
        scope: scope(),
        principal: p(1),
        state: LinkState::Degraded,
    };
    let Event::LinkStateChanged(msg) = to_event(&effect) else {
        panic!("expected the link-state arm");
    };
    assert_eq!(msg.principal, Some(proto::PrincipalId { value: 1 }));
    assert_eq!(msg.state, proto::LinkState::Degraded as i32);
}

#[test]
fn holder_link_loss_serializes_as_a_holder_link_lost_warning() {
    let effect = AuthorityEffect::HolderLinkLost {
        vehicle: vehicle(),
        scope: scope(),
        lost_holder: p(1),
        generation: g(4),
    };
    let Event::WarningRaised(msg) = to_event(&effect) else {
        panic!("expected the warning arm");
    };
    assert_eq!(msg.kind, proto::WarningKind::HolderLinkLost as i32);
    assert_eq!(msg.principal, Some(proto::PrincipalId { value: 1 }));
    assert_eq!(msg.generation, Some(proto::Generation { value: 4 }));
}

#[test]
fn unexpected_confirmations_carry_issuer_and_current_holder() {
    let cases = [
        (
            AuthorityWarning::UnexpectedIHave {
                by: p(2),
                current_holder: Some(p(1)),
            },
            proto::WarningKind::UnexpectedIHave,
        ),
        (
            AuthorityWarning::UnexpectedYouHave {
                by: p(2),
                current_holder: Some(p(1)),
            },
            proto::WarningKind::UnexpectedYouHave,
        ),
    ];
    for (raised, expected_kind) in cases {
        let effect = AuthorityEffect::WarningRaised {
            vehicle: vehicle(),
            scope: scope(),
            warning: raised,
        };
        let Event::WarningRaised(msg) = to_event(&effect) else {
            panic!("expected the warning arm");
        };
        assert_eq!(msg.kind, expected_kind as i32);
        assert_eq!(msg.principal, Some(proto::PrincipalId { value: 2 }));
        assert_eq!(msg.current_holder, Some(proto::PrincipalId { value: 1 }));
    }
}

#[test]
fn every_reject_reason_serializes_as_a_command_rejected_warning() {
    for reason in all_reject_reasons() {
        let effect = AuthorityEffect::CommandRejected {
            vehicle: vehicle(),
            scope: scope(),
            reason: reason.clone(),
        };
        let Event::WarningRaised(msg) = to_event(&effect) else {
            panic!("expected the warning arm for {reason:?}");
        };
        assert_eq!(
            msg.kind,
            proto::WarningKind::CommandRejected as i32,
            "wrong kind for {reason:?}"
        );
        assert!(!msg.detail.is_empty(), "empty detail for {reason:?}");
    }
}

#[test]
fn stale_accept_rejection_carries_both_generations() {
    let effect = AuthorityEffect::CommandRejected {
        vehicle: vehicle(),
        scope: scope(),
        reason: RejectReason::GenerationMismatch {
            supplied: g(1),
            current: g(2),
        },
    };
    let Event::WarningRaised(msg) = to_event(&effect) else {
        panic!("expected the warning arm");
    };
    assert_eq!(msg.generation, Some(proto::Generation { value: 2 }));
    assert_eq!(
        msg.detail,
        "stale accept fenced out: supplied generation 1, current generation 2"
    );
}

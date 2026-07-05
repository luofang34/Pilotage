//! Emergency override idempotency, override during a pending offer,
//! revoke/release fencing, and the v1 link-loss release policy (ADR-0010).

use core::time::Duration;

use pilotage_protocol::PrincipalId;

use crate::command::{AuthorityClass, AuthorityCommand, LinkState};
use crate::effect::{AuthorityEffect, FrameVerdict};
use crate::tests::{at, current_generation, engine_held_by, reason, scope, vehicle};

/// A repeat override by the same holder with the same class is idempotent: it
/// reports already-effective and does not advance the generation.
#[test]
fn override_is_idempotent_for_same_holder_and_class() {
    let alice = PrincipalId::new(10);
    let carol = PrincipalId::new(30);
    let (mut engine, _) = engine_held_by(alice);

    let first = engine.handle(
        AuthorityCommand::EmergencyOverride {
            vehicle: vehicle(),
            scope: scope(),
            by: carol,
            authority_class: AuthorityClass::Supervisor,
            reason: reason(),
        },
        at(1_000),
    );
    let gen_after_first = match first.as_slice() {
        [AuthorityEffect::EmergencyOverrideApplied { generation, .. }] => *generation,
        other => panic!("expected override applied, got {other:?}"),
    };

    let second = engine.handle(
        AuthorityCommand::EmergencyOverride {
            vehicle: vehicle(),
            scope: scope(),
            by: carol,
            authority_class: AuthorityClass::Supervisor,
            reason: reason(),
        },
        at(1_100),
    );
    match second.as_slice() {
        [
            AuthorityEffect::EmergencyOverrideAlreadyEffective {
                holder,
                authority_class,
                generation,
                ..
            },
        ] => {
            assert_eq!(*holder, carol);
            assert_eq!(*authority_class, AuthorityClass::Supervisor);
            assert_eq!(
                *generation, gen_after_first,
                "idempotent repeat must not advance the generation",
            );
        }
        other => panic!("expected already-effective, got {other:?}"),
    }
    assert_eq!(current_generation(&engine), gen_after_first);
}

/// A repeat override with a *different* class is not idempotent: it advances.
#[test]
fn override_with_different_class_advances() {
    let alice = PrincipalId::new(10);
    let carol = PrincipalId::new(30);
    let (mut engine, _) = engine_held_by(alice);
    let first = engine.handle(
        AuthorityCommand::EmergencyOverride {
            vehicle: vehicle(),
            scope: scope(),
            by: carol,
            authority_class: AuthorityClass::Supervisor,
            reason: reason(),
        },
        at(1_000),
    );
    let gen_after_first = match first.as_slice() {
        [AuthorityEffect::EmergencyOverrideApplied { generation, .. }] => *generation,
        other => panic!("expected override applied, got {other:?}"),
    };
    let escalated = engine.handle(
        AuthorityCommand::EmergencyOverride {
            vehicle: vehicle(),
            scope: scope(),
            by: carol,
            authority_class: AuthorityClass::Administrator,
            reason: reason(),
        },
        at(1_100),
    );
    match escalated.as_slice() {
        [AuthorityEffect::EmergencyOverrideApplied { generation, .. }] => {
            assert_ne!(*generation, gen_after_first);
        }
        other => panic!("expected a fresh override, got {other:?}"),
    }
}

/// Revoke empties the scope and immediately fences out the old holder's frame.
#[test]
fn revoke_advances_generation_and_fences() {
    let alice = PrincipalId::new(10);
    let (mut engine, gen_held) = engine_held_by(alice);
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::Accepted,
    );

    let revoked = engine.handle(
        AuthorityCommand::Revoke {
            vehicle: vehicle(),
            scope: scope(),
            authority_class: AuthorityClass::Administrator,
        },
        at(1_000),
    );
    let new_gen = match revoked.as_slice() {
        [
            AuthorityEffect::ScopeLeaseRevoked {
                previous_holder,
                generation,
                ..
            },
        ] => {
            assert_eq!(*previous_holder, Some(alice));
            *generation
        }
        other => panic!("expected revoke effect, got {other:?}"),
    };
    assert_ne!(new_gen, gen_held);
    // Scope is now unassigned: any frame is rejected for no holder.
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), new_gen),
        FrameVerdict::RejectedNoHolder,
    );
    // The old-generation frame is stale regardless.
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::RejectedNoHolder,
    );
}

/// Release by the holder empties the scope and fences out the old generation.
#[test]
fn release_advances_generation_and_fences() {
    let alice = PrincipalId::new(10);
    let (mut engine, gen_held) = engine_held_by(alice);
    let released = engine.handle(
        AuthorityCommand::Release {
            vehicle: vehicle(),
            scope: scope(),
            by: alice,
        },
        at(1_000),
    );
    let new_gen = match released.as_slice() {
        [
            AuthorityEffect::ScopeLeaseRevoked {
                previous_holder,
                generation,
                ..
            },
        ] => {
            assert_eq!(*previous_holder, Some(alice));
            *generation
        }
        other => panic!("expected release effect, got {other:?}"),
    };
    assert_ne!(new_gen, gen_held);
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), new_gen),
        FrameVerdict::RejectedNoHolder,
    );
}

/// Release by a non-holder is rejected and changes nothing.
#[test]
fn release_by_non_holder_is_rejected() {
    let alice = PrincipalId::new(10);
    let mallory = PrincipalId::new(99);
    let (mut engine, gen_held) = engine_held_by(alice);
    let rejected = engine.handle(
        AuthorityCommand::Release {
            vehicle: vehicle(),
            scope: scope(),
            by: mallory,
        },
        at(1_000),
    );
    assert!(matches!(
        rejected.as_slice(),
        [AuthorityEffect::CommandRejected { .. }]
    ));
    assert_eq!(current_generation(&engine), gen_held);
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::Accepted,
    );
}

/// Losing the effective holder's link releases the scope and advances the
/// generation (v1 policy), fencing out the lost holder immediately.
#[test]
fn link_lost_releases_scope() {
    let alice = PrincipalId::new(10);
    let (mut engine, gen_held) = engine_held_by(alice);

    // A degraded report keeps the hold.
    let degraded = engine.handle(
        AuthorityCommand::HolderLinkChanged {
            vehicle: vehicle(),
            scope: scope(),
            principal: alice,
            state: LinkState::Degraded,
        },
        at(1_000),
    );
    assert!(matches!(
        degraded.as_slice(),
        [AuthorityEffect::LinkStateChanged {
            state: LinkState::Degraded,
            ..
        }]
    ));
    assert_eq!(current_generation(&engine), gen_held);
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::Accepted,
    );

    // A lost report releases the scope.
    let lost = engine.handle(
        AuthorityCommand::HolderLinkChanged {
            vehicle: vehicle(),
            scope: scope(),
            principal: alice,
            state: LinkState::Lost,
        },
        at(1_100),
    );
    let new_gen = match lost.as_slice() {
        [
            AuthorityEffect::LinkStateChanged {
                state: LinkState::Lost,
                ..
            },
            AuthorityEffect::HolderLinkLost {
                lost_holder,
                generation,
                ..
            },
        ] => {
            assert_eq!(*lost_holder, alice);
            *generation
        }
        other => panic!("expected link-lost release, got {other:?}"),
    };
    assert_ne!(new_gen, gen_held);
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), new_gen),
        FrameVerdict::RejectedNoHolder,
    );
}

/// A link report about a principal that is not the effective holder is
/// recorded but drives no release.
#[test]
fn link_lost_for_non_holder_is_recorded_only() {
    let alice = PrincipalId::new(10);
    let bob = PrincipalId::new(20);
    let (mut engine, gen_held) = engine_held_by(alice);

    // Offer to Bob so Bob is a party but Alice is still the effective holder.
    let _ = engine.handle(
        AuthorityCommand::Offer {
            vehicle: vehicle(),
            scope: scope(),
            from: alice,
            to: bob,
            ttl: Duration::from_millis(500),
        },
        at(1_000),
    );
    let report = engine.handle(
        AuthorityCommand::HolderLinkChanged {
            vehicle: vehicle(),
            scope: scope(),
            principal: bob,
            state: LinkState::Lost,
        },
        at(1_100),
    );
    assert!(matches!(
        report.as_slice(),
        [AuthorityEffect::LinkStateChanged { .. }]
    ));
    // Alice still holds at the same generation.
    assert_eq!(current_generation(&engine), gen_held);
    assert_eq!(
        engine
            .scope_state(vehicle(), &scope())
            .expect("registered")
            .effective_holder(),
        Some(alice),
    );
}

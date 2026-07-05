//! The expiry-vs-accept race from ADR-0010 (a late accept self-fences to the
//! offerer without a prior `expire_due`, and the exact-deadline boundary) plus
//! an emergency override cancelling a pending offer.

use core::time::Duration;

use pilotage_protocol::PrincipalId;

use crate::command::{AuthorityClass, AuthorityCommand};
use crate::effect::{AuthorityEffect, FrameVerdict, RejectReason};
use crate::tests::{at, engine_held_by, reason, scope, vehicle};

/// An accept that arrives at or after the offer's `expires_at` self-fences:
/// even without a prior `expire_due`, it reverts the scope to the offerer and
/// never commits the transfer (ADR-0010 expiry-vs-accept race).
#[test]
fn accept_after_ttl_without_expire_due_reverts_to_offerer() {
    let alice = PrincipalId::new(10);
    let bob = PrincipalId::new(20);
    let (mut engine, gen_held) = engine_held_by(alice);
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
    let deadline = engine.next_deadline().expect("offer pending");

    // The host never calls expire_due; Bob's accept lands past the deadline.
    let late = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        at(deadline.as_nanos() + 1),
    );
    match late.as_slice() {
        [
            AuthorityEffect::ScopeTransferExpired {
                holder, generation, ..
            },
        ] => {
            assert_eq!(*holder, alice, "scope reverts to the offerer, not Bob");
            // No transfer occurred: the generation is unchanged.
            assert_eq!(*generation, gen_held);
        }
        other => panic!("expected expiry effect, got {other:?}"),
    }

    // Alice, not Bob, is the effective holder; her frames stay valid.
    assert_eq!(
        engine
            .scope_state(vehicle(), &scope())
            .expect("registered")
            .effective_holder(),
        Some(alice),
    );
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::Accepted,
    );
    // The offer is gone: a second accept finds nothing pending.
    let again = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        at(deadline.as_nanos() + 2),
    );
    assert!(matches!(
        again.as_slice(),
        [AuthorityEffect::CommandRejected {
            reason: RejectReason::NoPendingOffer,
            ..
        }]
    ));
}

/// An accept exactly at `expires_at` is treated as expired: `expire_due` fires
/// at `expires_at <= now`, so the accept path uses the same boundary and does
/// not commit.
#[test]
fn accept_exactly_at_expiry_does_not_commit() {
    let alice = PrincipalId::new(10);
    let bob = PrincipalId::new(20);
    let (mut engine, gen_held) = engine_held_by(alice);
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
    let deadline = engine.next_deadline().expect("offer pending");
    let at_deadline = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        deadline,
    );
    assert!(
        matches!(
            at_deadline.as_slice(),
            [AuthorityEffect::ScopeTransferExpired { holder, .. }] if *holder == alice
        ),
        "accept at the exact deadline expires rather than commits, got {at_deadline:?}",
    );
}

/// An emergency override during a pending offer cancels the offer and installs
/// the override holder.
#[test]
fn offer_cancelled_by_override() {
    let alice = PrincipalId::new(10);
    let bob = PrincipalId::new(20);
    let carol = PrincipalId::new(30);
    let (mut engine, gen_held) = engine_held_by(alice);
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
    assert!(engine.next_deadline().is_some());

    let overridden = engine.handle(
        AuthorityCommand::EmergencyOverride {
            vehicle: vehicle(),
            scope: scope(),
            by: carol,
            authority_class: AuthorityClass::Supervisor,
            reason: reason(),
        },
        at(1_100),
    );
    let new_gen = match overridden.as_slice() {
        [
            AuthorityEffect::EmergencyOverrideApplied {
                previous_holder,
                holder,
                generation,
                ..
            },
        ] => {
            assert_eq!(*previous_holder, Some(alice));
            assert_eq!(*holder, carol);
            *generation
        }
        other => panic!("expected override effect, got {other:?}"),
    };
    assert_ne!(new_gen, gen_held);
    // The offer is gone: no deadline, and a late accept finds nothing pending.
    assert!(engine.next_deadline().is_none());
    let late_accept = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        at(1_200),
    );
    assert!(matches!(
        late_accept.as_slice(),
        [AuthorityEffect::CommandRejected {
            reason: RejectReason::NoPendingOffer,
            ..
        }]
    ));
}

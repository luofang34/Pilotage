//! Normal handover, its atomic-commit fencing, and the offer/accept edge
//! cases from ADR-0010 (wrong generation, duplicate accept, accept after a
//! host-driven expiry). The expiry-vs-accept race and override-cancels-offer
//! scenarios live in `handover_expiry`.

use core::time::Duration;

use pilotage_protocol::PrincipalId;

use crate::command::AuthorityCommand;
use crate::effect::{AuthorityEffect, FrameVerdict, RejectReason};
use crate::tests::{at, current_generation, engine_held_by, scope, vehicle};

/// Alice offers, Bob accepts: the commit advances the generation and fences
/// Alice's old frames out immediately.
#[test]
fn normal_handover_commits_on_accept() {
    let alice = PrincipalId::new(10);
    let bob = PrincipalId::new(20);
    let (mut engine, gen_held) = engine_held_by(alice);

    let offered = engine.handle(
        AuthorityCommand::Offer {
            vehicle: vehicle(),
            scope: scope(),
            from: alice,
            to: bob,
            ttl: Duration::from_millis(500),
        },
        at(1_000),
    );
    // The offer does not advance the generation; Alice remains effective.
    match offered.as_slice() {
        [AuthorityEffect::ScopeTransferOffered { generation, .. }] => {
            assert_eq!(*generation, gen_held);
        }
        other => panic!("expected offer effect, got {other:?}"),
    }
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::Accepted,
        "offerer's frames stay valid while the offer is pending",
    );

    let committed = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        at(1_100),
    );
    let new_gen = match committed.as_slice() {
        [
            AuthorityEffect::ScopeTransferCommitted {
                from,
                to,
                generation,
                ..
            },
        ] => {
            assert_eq!(*from, alice);
            assert_eq!(*to, bob);
            *generation
        }
        other => panic!("expected commit effect, got {other:?}"),
    };
    assert_ne!(new_gen, gen_held, "commit advances the generation");

    // Fencing: Alice's frame at the old generation is now stale.
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::RejectedStaleGeneration { current: new_gen },
    );
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), new_gen),
        FrameVerdict::Accepted,
    );
}

/// The full positive three-call exchange audits without altering committed
/// state: confirmations never gate the transfer.
#[test]
fn confirmations_do_not_gate_transfer() {
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
    let _ = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        at(1_100),
    );
    let after_commit = current_generation(&engine);

    // Bob: "I have control" — expected, benign, no generation change.
    let i_have = engine.handle(
        AuthorityCommand::ConfirmIHave {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
        },
        at(1_200),
    );
    assert!(matches!(
        i_have.as_slice(),
        [AuthorityEffect::LinkStateChanged { .. }]
    ));

    // Alice: "you have control" — the previous holder's callout, benign.
    let you_have = engine.handle(
        AuthorityCommand::ConfirmYouHave {
            vehicle: vehicle(),
            scope: scope(),
            by: alice,
        },
        at(1_300),
    );
    assert!(matches!(
        you_have.as_slice(),
        [AuthorityEffect::LinkStateChanged { .. }]
    ));
    assert_eq!(current_generation(&engine), after_commit);
}

/// A contradictory "I have control" from the displaced holder warns and does
/// not change state.
#[test]
fn contradictory_confirmation_warns_only() {
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
    let _ = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        at(1_100),
    );
    let gen_before = current_generation(&engine);
    let warned = engine.handle(
        AuthorityCommand::ConfirmIHave {
            vehicle: vehicle(),
            scope: scope(),
            by: alice,
        },
        at(1_200),
    );
    assert!(matches!(
        warned.as_slice(),
        [AuthorityEffect::WarningRaised { .. }]
    ));
    assert_eq!(current_generation(&engine), gen_before);
}

/// Accept with a stale `expected_generation` is fenced out; no transfer.
#[test]
fn accept_with_wrong_generation_is_rejected() {
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
    let wrong = gen_held.next();
    let rejected = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: wrong,
        },
        at(1_100),
    );
    match rejected.as_slice() {
        [
            AuthorityEffect::CommandRejected {
                reason: RejectReason::GenerationMismatch { supplied, current },
                ..
            },
        ] => {
            assert_eq!(*supplied, wrong);
            assert_eq!(*current, gen_held);
        }
        other => panic!("expected generation mismatch rejection, got {other:?}"),
    }
    // Alice still holds; the offer is still pending at the same generation.
    assert_eq!(current_generation(&engine), gen_held);
}

/// A duplicate accept after a successful commit finds no pending offer.
#[test]
fn duplicate_accept_is_rejected() {
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
    let _ = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        at(1_100),
    );
    let committed_gen = current_generation(&engine);

    let dup = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: committed_gen,
        },
        at(1_200),
    );
    assert!(matches!(
        dup.as_slice(),
        [AuthorityEffect::CommandRejected {
            reason: RejectReason::NoPendingOffer,
            ..
        }]
    ));
    assert_eq!(current_generation(&engine), committed_gen);
}

/// An offer expires back to the offerer; a late accept afterward is rejected.
#[test]
fn accept_after_expiry_is_rejected() {
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
    // The offer expires at 1_000 + 500ms.
    let deadline = engine.next_deadline().expect("offer pending");
    let expired = engine.expire_due(deadline);
    match expired.as_slice() {
        [
            AuthorityEffect::ScopeTransferExpired {
                holder, generation, ..
            },
        ] => {
            assert_eq!(*holder, alice);
            // Expiry does not advance the generation: no transfer occurred.
            assert_eq!(*generation, gen_held);
        }
        other => panic!("expected expiry effect, got {other:?}"),
    }

    let late = engine.handle(
        AuthorityCommand::Accept {
            vehicle: vehicle(),
            scope: scope(),
            by: bob,
            expected_generation: gen_held,
        },
        at(deadline.as_nanos() + 1),
    );
    assert!(matches!(
        late.as_slice(),
        [AuthorityEffect::CommandRejected {
            reason: RejectReason::NoPendingOffer,
            ..
        }]
    ));
    // Alice retained control across the whole sequence.
    assert_eq!(
        engine
            .scope_state(vehicle(), &scope())
            .expect("registered")
            .effective_holder(),
        Some(alice),
    );
}

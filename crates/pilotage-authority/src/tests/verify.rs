//! The [`FrameVerdict`] matrix: unknown scope, no holder, stale generation,
//! accepted, and the pending-offer case where the offerer's frames stay valid
//! (ADR-0006).

use core::time::Duration;

use pilotage_protocol::{Generation, PrincipalId, ScopeId, VehicleId};

use crate::command::AuthorityCommand;
use crate::effect::FrameVerdict;
use crate::engine::AuthorityEngine;
use crate::tests::{at, current_generation, engine_held_by, engine_registered, scope, vehicle};

/// An unregistered `(vehicle, scope)` is unknown regardless of generation.
#[test]
fn verify_unknown_scope() {
    let engine = AuthorityEngine::new();
    assert_eq!(
        engine.verify_frame(VehicleId::new(7), &ScopeId::new("nope"), Generation::new(0)),
        FrameVerdict::RejectedUnknownScope,
    );
}

/// A registered but unassigned scope rejects for no holder.
#[test]
fn verify_no_holder() {
    let engine = engine_registered();
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), Generation::new(0)),
        FrameVerdict::RejectedNoHolder,
    );
}

/// A held scope accepts the current generation and reports staleness with the
/// current generation for any other.
#[test]
fn verify_held_matrix() {
    let alice = PrincipalId::new(10);
    let (engine, gen_held) = engine_held_by(alice);

    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::Accepted,
    );
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held.next()),
        FrameVerdict::RejectedStaleGeneration { current: gen_held },
    );
    // A generation below current is also stale, and the verdict always carries
    // the true current generation.
    let below = Generation::new(gen_held.as_u64().wrapping_sub(1));
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), below),
        FrameVerdict::RejectedStaleGeneration { current: gen_held },
    );
}

/// During a pending offer the offerer's frames at the current generation stay
/// valid; the recipient cannot yet send valid frames.
#[test]
fn verify_during_pending_offer() {
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
    // Generation is unchanged while offered, and frames at it are accepted.
    assert_eq!(current_generation(&engine), gen_held);
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held),
        FrameVerdict::Accepted,
    );
    // The generation the recipient would use after a commit is not yet valid.
    assert_eq!(
        engine.verify_frame(vehicle(), &scope(), gen_held.next()),
        FrameVerdict::RejectedStaleGeneration { current: gen_held },
    );
}

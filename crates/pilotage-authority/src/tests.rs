//! Unit and property tests for the authority engine.
//!
//! Because the engine is sans-IO (ADR-0002), every scenario in ADR-0010 is a
//! table-driven test: construct an engine, feed commands with explicit `now`,
//! and assert the ordered effects and resulting fencing state.

#![allow(clippy::expect_used, clippy::panic)]

mod handover;
mod handover_expiry;
mod override_link;
mod properties;
mod verify;

use pilotage_protocol::{Generation, PrincipalId, ScopeId, VehicleId};
use pilotage_timing::MonoTimestamp;

use crate::command::{AuthorityCommand, OverrideReason};
use crate::effect::AuthorityEffect;
use crate::engine::AuthorityEngine;

/// A fixed vehicle for single-scope scenarios.
fn vehicle() -> VehicleId {
    VehicleId::new(1)
}

/// A fixed scope for single-scope scenarios.
fn scope() -> ScopeId {
    ScopeId::new("vehicle.motion")
}

/// A monotonic timestamp `nanos` into the test epoch.
fn at(nanos: u64) -> MonoTimestamp {
    MonoTimestamp::from_nanos(nanos)
}

/// Builds an engine with the standard `(vehicle, scope)` registered and held
/// by `holder`, returning the engine and the current generation.
fn engine_held_by(holder: PrincipalId) -> (AuthorityEngine, Generation) {
    let mut engine = engine_registered();
    let effects = engine.handle(
        AuthorityCommand::Grant {
            vehicle: vehicle(),
            scope: scope(),
            to: holder,
        },
        at(0),
    );
    let generation = match effects.as_slice() {
        [AuthorityEffect::ScopeLeaseGranted { generation, .. }] => *generation,
        other => panic!("expected a single grant effect, got {other:?}"),
    };
    (engine, generation)
}

/// Registers the standard scope on a fresh engine and returns it.
fn engine_registered() -> AuthorityEngine {
    let mut engine = AuthorityEngine::new();
    let _ = engine.handle(
        AuthorityCommand::RegisterScope {
            vehicle: vehicle(),
            scope: scope(),
        },
        at(0),
    );
    engine
}

/// The current fencing generation of the standard scope.
fn current_generation(engine: &AuthorityEngine) -> Generation {
    engine
        .scope_state(vehicle(), &scope())
        .expect("scope registered")
        .generation
}

/// A convenience [`OverrideReason`].
fn reason() -> OverrideReason {
    OverrideReason::new("test override")
}

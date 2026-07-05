//! Property tests over arbitrary command sequences (ADR-0010).
//!
//! Invariants asserted for every reachable state:
//! - at most one effective holder per scope (structural, but checked against
//!   the effects the engine emitted);
//! - the fencing generation never decreases across a bounded sequence (it can
//!   only wrap on `u64` overflow, unreachable here);
//! - no emitted effect sequence ever implies two distinct principals holding
//!   the same scope simultaneously.

use core::time::Duration;

use proptest::prelude::*;

use pilotage_protocol::{Generation, PrincipalId, ScopeId, VehicleId};

use crate::command::{AuthorityClass, AuthorityCommand, LinkState, OverrideReason};
use crate::effect::AuthorityEffect;
use crate::engine::AuthorityEngine;

/// The small vehicle used by the property model.
fn model_vehicle() -> VehicleId {
    VehicleId::new(1)
}

/// The two scopes the property model exercises.
fn model_scope(index: usize) -> ScopeId {
    ScopeId::new(if index == 0 {
        "vehicle.motion"
    } else {
        "vehicle.camera"
    })
}

/// A principal drawn from a small fixed pool.
fn principal(index: u8) -> PrincipalId {
    PrincipalId::new(u64::from(index))
}

/// Strategy over commands targeting one of two scopes and three principals.
///
/// The generation supplied to `Accept` is deliberately drawn from a tiny range
/// so accepts sometimes match and sometimes are fenced out.
fn command_strategy() -> impl Strategy<Value = AuthorityCommand> {
    let scope_idx = 0usize..2;
    let princ = 0u8..3;
    let princ2 = 0u8..3;
    let gen_value = 0u64..4;
    let class = prop_oneof![
        Just(AuthorityClass::Operator),
        Just(AuthorityClass::Supervisor),
        Just(AuthorityClass::Administrator),
        Just(AuthorityClass::Automation),
    ];
    let link = prop_oneof![
        Just(LinkState::Nominal),
        Just(LinkState::Degraded),
        Just(LinkState::Lost),
    ];

    (scope_idx, princ, princ2, gen_value, class, link).prop_flat_map(|(si, p, p2, g, cls, lk)| {
        let scope = move || model_scope(si);
        let a = principal(p);
        let b = principal(p2);
        prop_oneof![
            Just(AuthorityCommand::Grant {
                vehicle: model_vehicle(),
                scope: scope(),
                to: a,
            }),
            Just(AuthorityCommand::Offer {
                vehicle: model_vehicle(),
                scope: scope(),
                from: a,
                to: b,
                ttl: Duration::from_millis(10),
            }),
            Just(AuthorityCommand::Accept {
                vehicle: model_vehicle(),
                scope: scope(),
                by: b,
                expected_generation: Generation::new(g),
            }),
            Just(AuthorityCommand::ConfirmIHave {
                vehicle: model_vehicle(),
                scope: scope(),
                by: a,
            }),
            Just(AuthorityCommand::ConfirmYouHave {
                vehicle: model_vehicle(),
                scope: scope(),
                by: a,
            }),
            Just(AuthorityCommand::Release {
                vehicle: model_vehicle(),
                scope: scope(),
                by: a,
            }),
            Just(AuthorityCommand::Revoke {
                vehicle: model_vehicle(),
                scope: scope(),
                authority_class: cls,
            }),
            Just(AuthorityCommand::EmergencyOverride {
                vehicle: model_vehicle(),
                scope: scope(),
                by: a,
                authority_class: cls,
                reason: OverrideReason::new("prop"),
            }),
            Just(AuthorityCommand::HolderLinkChanged {
                vehicle: model_vehicle(),
                scope: scope(),
                principal: a,
                state: lk,
            }),
        ]
    })
}

/// The holder each effect implies for its scope after being applied.
///
/// `Some(Some(p))` means "now held by `p`", `Some(None)` means "now
/// unassigned", and `None` means "this effect does not change the holder".
fn implied_holder(effect: &AuthorityEffect) -> Option<Option<PrincipalId>> {
    match effect {
        AuthorityEffect::ScopeLeaseGranted { holder, .. } => Some(Some(*holder)),
        AuthorityEffect::ScopeTransferCommitted { to, .. } => Some(Some(*to)),
        AuthorityEffect::EmergencyOverrideApplied { holder, .. } => Some(Some(*holder)),
        AuthorityEffect::ScopeTransferExpired { holder, .. } => Some(Some(*holder)),
        AuthorityEffect::ScopeLeaseRevoked { .. } | AuthorityEffect::HolderLinkLost { .. } => {
            Some(None)
        }
        _ => None,
    }
}

/// The raw generation the engine currently reports for a scope, or 0 if the
/// scope is not registered.
fn raw_generation(engine: &AuthorityEngine, scope: &ScopeId) -> u64 {
    engine
        .scope_state(model_vehicle(), scope)
        .map_or(0, |state| state.generation.as_u64())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn arbitrary_sequences_preserve_invariants(
        commands in proptest::collection::vec(command_strategy(), 0..40),
    ) {
        let mut engine = AuthorityEngine::new();
        for idx in 0..2 {
            let _ = engine.handle(
                AuthorityCommand::RegisterScope {
                    vehicle: model_vehicle(),
                    scope: model_scope(idx),
                },
                pilotage_timing::MonoTimestamp::from_nanos(0),
            );
        }

        let mut now_nanos: u64 = 1;
        let mut last_gen = [0u64; 2];

        for command in commands {
            let now = pilotage_timing::MonoTimestamp::from_nanos(now_nanos);
            now_nanos = now_nanos.wrapping_add(1_000_000);

            // Occasionally fire expiries so pending offers are exercised.
            let _ = engine.expire_due(now);

            let effects = engine.handle(command, now);

            // A command always produces at least one effect.
            prop_assert!(!effects.is_empty());

            for (scope_idx, last) in last_gen.iter_mut().enumerate() {
                let scope = model_scope(scope_idx);

                // Generation is monotonically non-decreasing (no wrap here).
                let raw_gen = raw_generation(&engine, &scope);
                prop_assert!(
                    raw_gen >= *last,
                    "generation went backwards: {} -> {}",
                    *last,
                    raw_gen,
                );
                *last = raw_gen;

                // At most one effective holder is structurally guaranteed;
                // assert it matches the last mutating effect for this scope.
                if let Some(state) = engine.scope_state(model_vehicle(), &scope) {
                    let effective = state.effective_holder();
                    let implied = effects
                        .iter()
                        .filter(|e| effect_targets(e, &scope))
                        .filter_map(implied_holder)
                        .next_back();
                    if let Some(implied_holder) = implied {
                        prop_assert_eq!(
                            effective,
                            implied_holder,
                            "effective holder disagrees with emitted effect",
                        );
                    }
                }
            }
        }
    }
}

/// Whether an effect concerns the given scope.
fn effect_targets(effect: &AuthorityEffect, target: &ScopeId) -> bool {
    let scope = match effect {
        AuthorityEffect::ScopeRegistered { scope, .. }
        | AuthorityEffect::ScopeLeaseGranted { scope, .. }
        | AuthorityEffect::ScopeTransferOffered { scope, .. }
        | AuthorityEffect::ScopeTransferCommitted { scope, .. }
        | AuthorityEffect::ScopeTransferExpired { scope, .. }
        | AuthorityEffect::ScopeLeaseRevoked { scope, .. }
        | AuthorityEffect::EmergencyOverrideApplied { scope, .. }
        | AuthorityEffect::EmergencyOverrideAlreadyEffective { scope, .. }
        | AuthorityEffect::LinkStateChanged { scope, .. }
        | AuthorityEffect::HolderLinkLost { scope, .. }
        | AuthorityEffect::WarningRaised { scope, .. }
        | AuthorityEffect::CommandRejected { scope, .. } => scope,
    };
    scope == target
}

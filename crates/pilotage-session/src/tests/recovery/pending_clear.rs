//! Pending-clear invalidation across authority-generation changes: a
//! committed handover or an emergency override reverts a `ClearPending`
//! scope to `Engaged`, so a stale clear can never un-latch the scope for a
//! new holder without its own neutral activation.

use super::*;

fn engine_with_a_pending_clear() -> (SessionEngine, Generation) {
    let (mut engine, client, session, generation) = engaged_then_regranted();
    let neutral = engine.handle_client_message(
        client,
        DomainEnvelope::Frame(neutral_frame(
            session,
            generation,
            SequenceNum::new(3),
            MonoTimestamp::from_nanos(230),
        )),
        MonoTimestamp::from_nanos(230),
    );
    assert_eq!(
        cleared(&neutral),
        1,
        "the neutral activation requests the clear"
    );
    // While the clear is pending (unconfirmed), each tick re-emits its retry.
    let retry = engine.handle_tick(MonoTimestamp::from_nanos(231));
    assert_eq!(cleared(&retry), 1, "a pending clear is retried each tick");
    (engine, generation)
}

#[test]
fn a_committed_handover_invalidates_a_pending_clear() {
    let (mut engine, _generation) = engine_with_a_pending_clear();

    // A committed handover installs a new holder and advances the generation
    // with NO new engagement. It must invalidate the pending clear, so the
    // stale retry stops and the new holder has to re-demonstrate neutral
    // activation — a clear from the old generation can never un-latch the scope
    // for the new holder (ADR-0010).
    let handover = engine.apply_authority_effect_for_test(
        AuthorityEffect::ScopeTransferCommitted {
            vehicle: VEHICLE,
            scope: motion(),
            from: PrincipalId::new(1),
            to: PrincipalId::new(2),
            generation: Generation::new(999),
        },
        MonoTimestamp::from_nanos(232),
    );
    assert_eq!(cleared(&handover), 0, "the handover itself is not a clear");

    let after = engine.handle_tick(MonoTimestamp::from_nanos(233));
    assert_eq!(
        cleared(&after),
        0,
        "the handover invalidated the pending clear; no stale clear crosses the generation"
    );
}

#[test]
fn an_emergency_override_invalidates_a_pending_clear() {
    let (mut engine, _generation) = engine_with_a_pending_clear();

    // An emergency override seizes the scope at a new generation with no new
    // engagement — the same race as a handover, and the same invalidation.
    let overridden = engine.apply_authority_effect_for_test(
        AuthorityEffect::EmergencyOverrideApplied {
            vehicle: VEHICLE,
            scope: motion(),
            previous_holder: Some(PrincipalId::new(1)),
            holder: PrincipalId::new(2),
            authority_class: AuthorityClass::Supervisor,
            reason: OverrideReason::new("recovery race test"),
            generation: Generation::new(999),
        },
        MonoTimestamp::from_nanos(232),
    );
    assert_eq!(
        cleared(&overridden),
        0,
        "the override itself is not a clear"
    );

    let after = engine.handle_tick(MonoTimestamp::from_nanos(233));
    assert_eq!(
        cleared(&after),
        0,
        "the override invalidated the pending clear; recovery needs fresh neutral activation"
    );
}

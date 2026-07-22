//! `LinkLossState` lifecycle unit tests (ADR-0010): the `Engaged →
//! ClearPending → Cleared` state machine, generation-gated confirmation, and
//! the holder-change invalidation that stops a stale clear from crossing a
//! generation.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::{Generation, ScopeId, VehicleId};

use super::LinkLossState;

const V: VehicleId = VehicleId::new(1);

fn state() -> LinkLossState {
    LinkLossState {
        selected: Vec::new(),
        engaged: Vec::new(),
    }
}

fn motion() -> ScopeId {
    ScopeId::new("vehicle.motion")
}

#[test]
fn a_holder_change_invalidates_a_pending_clear_so_a_new_generation_is_required() {
    let mut link_loss = state();
    link_loss.engage_scope(V, &motion());
    assert!(link_loss.is_awaiting_activation(V, &motion()));

    // A neutral activation at generation 5 moves the scope to ClearPending.
    assert!(link_loss.begin_clear_pending(V, &motion(), Generation::new(5)));
    assert!(!link_loss.is_awaiting_activation(V, &motion()));
    assert_eq!(
        link_loss.pending_clears(),
        vec![(V, motion(), Generation::new(5))]
    );
    // Already pending: a second activation does not re-transition.
    assert!(!link_loss.begin_clear_pending(V, &motion(), Generation::new(5)));

    // A holder change (committed handover / emergency override) invalidates the
    // pending clear: it reverts to Engaged, so no stale retry survives.
    link_loss.invalidate_pending(V, &motion());
    assert!(
        link_loss.is_awaiting_activation(V, &motion()),
        "back to Engaged"
    );
    assert!(
        link_loss.pending_clears().is_empty(),
        "no stale retry survives"
    );

    // A confirm at the STALE generation now does nothing — the clear that was
    // in flight can never un-latch the scope across the generation.
    assert!(!link_loss.confirm_cleared(V, &motion(), Generation::new(5)));
    assert!(
        link_loss.is_awaiting_activation(V, &motion()),
        "still engaged, still latched"
    );

    // Recovery requires a FRESH neutral activation at the NEW generation.
    assert!(link_loss.begin_clear_pending(V, &motion(), Generation::new(6)));
    assert!(
        link_loss.confirm_cleared(V, &motion(), Generation::new(6)),
        "the fresh clear at the new generation confirms"
    );
    assert!(link_loss.pending_clears().is_empty());
    assert!(
        link_loss.state(V, &motion()).is_none(),
        "cleared: no longer tracked"
    );
}

#[test]
fn confirm_is_generation_gated() {
    let mut link_loss = state();
    link_loss.engage_scope(V, &motion());
    link_loss.begin_clear_pending(V, &motion(), Generation::new(5));

    // A confirm at the wrong generation is rejected — no ack, still pending.
    assert!(!link_loss.confirm_cleared(V, &motion(), Generation::new(4)));
    assert_eq!(
        link_loss.pending_clears(),
        vec![(V, motion(), Generation::new(5))]
    );
    // The matching generation confirms and drops it.
    assert!(link_loss.confirm_cleared(V, &motion(), Generation::new(5)));
    assert!(link_loss.state(V, &motion()).is_none());
}

#[test]
fn a_re_engage_discards_a_pending_clear() {
    let mut link_loss = state();
    link_loss.engage_scope(V, &motion());
    link_loss.begin_clear_pending(V, &motion(), Generation::new(5));

    // A new holder loss re-engages the scope, discarding the pending clear so
    // the new loss recovers through a fresh neutral activation.
    link_loss.engage_scope(V, &motion());
    assert!(link_loss.is_awaiting_activation(V, &motion()));
    assert!(link_loss.pending_clears().is_empty());
}

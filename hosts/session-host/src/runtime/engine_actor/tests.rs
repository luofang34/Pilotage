//! The engine actor must actually enact link-loss actions on the adapter —
//! releasing a lease without calling `set_link_loss_policy` would leave the
//! vehicle on its last command (ADR-0008) — and a failed enactment is a
//! counted fail-closed fault, never a silent no-op.
#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossEnactError,
    LinkLossPolicy, RejectReason, ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch,
    VehicleAdapter, VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{
    ClientHello, ControlPayload, Generation, LeaseRelease, LeaseRequest, LogicalAxisId, ScopeId,
    ScopedControlFrame, SequenceNum, SessionId, VehicleId,
};
use pilotage_session::{
    ClientKey, DomainEnvelope, LinkLossTrigger, OutboundMessage, SessionAction, SessionConfig,
    SessionEngine, SessionOutcome,
};
use pilotage_timing::{MonoTimestamp, SimTick, StalenessPolicy};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::EngineActor;
use crate::runtime::connection::ToConnection;
use crate::runtime::registry::OUTBOUND_QUEUE_CAPACITY;

const VEHICLE: VehicleId = VehicleId::new(1);
const MOTION: &str = "vehicle.motion";

// Shared fixtures live in a sibling submodule to stay under the file-size
// gate; a child module reads this parent's imports via `use super::*`.
mod adapter_rejections;
mod fixtures;
mod reliable_actions;
use fixtures::*;

#[test]
fn engage_link_loss_neutralizes_the_adapter() {
    let mut actor = actor();
    actor.enact(SessionOutcome {
        actions: vec![engage_action()],
        dropped: 0,
    });
    assert_eq!(
        actor.adapter.link_loss_calls,
        vec![(
            VEHICLE,
            ScopeId::new(MOTION),
            Some(LinkLossPolicy::Neutralize)
        )],
        "EngageLinkLoss must call set_link_loss_policy(scope, Some(Neutralize))"
    );
    assert_eq!(actor.link_loss_enact_failures, 0);
}

#[test]
fn a_failed_enactment_is_a_counted_fault() {
    let mut actor = actor();
    actor.adapter.fail_enactment = true;
    actor.enact(SessionOutcome {
        actions: vec![engage_action()],
        dropped: 0,
    });
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "a refused policy change must be counted, never silent"
    );
}

#[test]
fn a_failed_clear_leaves_the_scope_suppressed() {
    let mut actor = actor();

    // Even a REFUSED engage records the latch, so the scope is suppressed
    // (ADR-0008 fail-closed): apply_control rejects it.
    actor.adapter.fail_enactment = true;
    actor.enact(SessionOutcome {
        actions: vec![engage_action()],
        dropped: 0,
    });
    assert_eq!(
        actor
            .adapter
            .apply_control(&motion_control_frame())
            .disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "a fenced engage suppresses control even when its actuation was refused"
    );

    // A clear the adapter REFUSES must NOT return the scope to control — the
    // latch drops only on Ok.
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        actor
            .adapter
            .apply_control(&motion_control_frame())
            .disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged),
        "a refused clear leaves the scope suppressed"
    );

    // A clear the adapter ACCEPTS returns the scope to normal control.
    actor.adapter.fail_enactment = false;
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        actor
            .adapter
            .apply_control(&motion_control_frame())
            .disposition,
        Disposition::Accepted,
        "a successful clear returns the scope to control"
    );
}

#[test]
fn clear_link_loss_returns_the_scope_to_normal_control() {
    let mut actor = actor();
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        actor.adapter.link_loss_calls,
        vec![(VEHICLE, ScopeId::new(MOTION), None)],
        "ClearLinkLoss must call set_link_loss_policy(scope, None)"
    );
}

#[test]
fn recovery_acks_once_when_the_adapter_confirms_the_clear() {
    let mut actor = actor();
    let mut client = register_client(&mut actor);
    let (session, generation) = drive_to_regranted(&mut actor);
    let _ = authority_messages(&mut client); // drop grant/release/re-grant broadcasts

    let recovered = actor.engine.handle_client_message(
        ClientKey::new(1),
        neutral_frame(session, generation, 1),
        MonoTimestamp::from_nanos(0),
    );
    actor.enact(recovered);
    assert_eq!(
        authority_messages(&mut client),
        1,
        "a confirmed recovery broadcasts exactly one LinkLossCleared ack"
    );
    assert!(
        actor
            .adapter
            .link_loss_calls
            .contains(&(VEHICLE, ScopeId::new(MOTION), None)),
        "the adapter's motion latch was cleared"
    );
}

#[test]
fn a_failed_clear_emits_no_ack_and_counts_the_fault() {
    let mut actor = actor();
    actor.adapter.fail_enactment = true;
    let mut client = register_client(&mut actor);
    actor.enact(SessionOutcome {
        actions: vec![clear_action(MOTION)],
        dropped: 0,
    });
    assert_eq!(
        authority_messages(&mut client),
        0,
        "a clear the adapter refused must NOT ack — the client keeps neutralizing"
    );
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "a refused clear is a counted fail-closed fault, never silent"
    );
}

#[test]
fn a_refused_clear_is_retried_by_the_engine_and_acks_once_when_it_takes() {
    let mut actor = actor();
    let mut client = register_client(&mut actor);
    let (session, generation) = drive_to_regranted(&mut actor);
    let _ = authority_messages(&mut client);

    // The clear is refused: no ack, and the engine holds the scope pending
    // (fail-closed) rather than dropping it, so recovery is not stranded.
    actor.adapter.fail_enactment = true;
    let recovered = actor.engine.handle_client_message(
        ClientKey::new(1),
        neutral_frame(session, generation, 1),
        MonoTimestamp::from_nanos(0),
    );
    actor.enact(recovered);
    assert_eq!(
        authority_messages(&mut client),
        0,
        "a refused clear does not ack"
    );
    assert_eq!(actor.link_loss_enact_failures, 1, "the refusal is counted");

    // The adapter recovers; the engine re-emits the clear on the next tick, it
    // takes, and the ack fires — exactly once.
    actor.adapter.fail_enactment = false;
    let tick = actor.engine.handle_tick(MonoTimestamp::from_nanos(1));
    actor.enact(tick);
    assert_eq!(
        authority_messages(&mut client),
        1,
        "the engine-driven retry acks once when the adapter takes the clear"
    );

    // A further tick neither re-clears nor re-acks: the scope is Cleared.
    let tick = actor.engine.handle_tick(MonoTimestamp::from_nanos(2));
    actor.enact(tick);
    assert_eq!(
        authority_messages(&mut client),
        0,
        "exactly one ack across the whole recovery"
    );
}

#[test]
fn a_persistently_refused_clear_is_counted_once_not_per_tick() {
    let mut actor = actor();
    let mut client = register_client(&mut actor);
    let (session, generation) = drive_to_regranted(&mut actor);
    let _ = authority_messages(&mut client);

    // The first (non-retry) clear attempt is refused: counted once.
    actor.adapter.fail_enactment = true;
    let recovered = actor.engine.handle_client_message(
        ClientKey::new(1),
        neutral_frame(session, generation, 1),
        MonoTimestamp::from_nanos(0),
    );
    actor.enact(recovered);
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "the first refusal is counted once"
    );

    // The engine re-emits the clear every tick; 50 refused RETRIES must add no
    // further faults (no 100 Hz counter/log storm).
    for tick_at in 1..=50 {
        let tick = actor.engine.handle_tick(MonoTimestamp::from_nanos(tick_at));
        actor.enact(tick);
    }
    assert_eq!(
        actor.link_loss_enact_failures, 1,
        "50 refused retries add no faults — counted once, not per tick"
    );
    assert_eq!(
        authority_messages(&mut client),
        0,
        "still no ack while the clear stays refused"
    );
}

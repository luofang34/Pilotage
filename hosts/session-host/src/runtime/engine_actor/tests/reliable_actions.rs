//! Exactly-once delivery for correlated discrete actions (CTRL-01): a
//! retransmitted action id executes once at the adapter and replays its
//! recorded result; uncorrelated actions (id 0) pass through untouched.

use super::fixtures::{actor, motion_control_frame, register_client};
use super::*;

/// A typed frame carrying only an ARM action with the given correlation id
/// (id 0 = uncorrelated).
fn arm_frame(action_id: u32) -> ScopedControlFrame {
    let mut frame = motion_control_frame();
    frame.payload = pilotage_protocol::ControlPayload {
        axes: vec![],
        edges: vec![],
    };
    frame.actions = vec![pilotage_protocol::ControlAction::Arm];
    frame.action_ids = vec![action_id];
    frame
}

/// Counts the reliable session-stream messages queued for the client — each
/// `ControlActionResult` rides one.
fn bootstrap_messages(receiver: &mut mpsc::Receiver<ToConnection>) -> usize {
    let mut count = 0;
    while let Ok(message) = receiver.try_recv() {
        if matches!(message, ToConnection::BootstrapMessage(_)) {
            count += 1;
        }
    }
    count
}

#[test]
fn a_retransmitted_action_executes_once_and_replays_its_result() {
    let mut actor = actor();
    let mut receiver = register_client(&mut actor);
    let client = ClientKey::new(1);

    actor.apply_to_adapter(client, arm_frame(7));
    assert_eq!(
        actor.adapter.applied_actions,
        vec![vec![pilotage_protocol::ControlAction::Arm]],
        "the first delivery reaches the adapter"
    );
    assert_eq!(bootstrap_messages(&mut receiver), 1, "one result answered");

    // The result datagram raced the next frame: the sender retransmits the
    // same correlation id. The adapter must NOT execute a second arm, but
    // the sender still gets its answer.
    actor.apply_to_adapter(client, arm_frame(7));
    assert_eq!(
        actor.adapter.applied_actions.len(),
        1,
        "a pure retransmission never reaches the adapter"
    );
    assert_eq!(
        bootstrap_messages(&mut receiver),
        1,
        "the cached result is replayed"
    );

    // A NEW press (fresh id) executes again.
    actor.apply_to_adapter(client, arm_frame(8));
    assert_eq!(actor.adapter.applied_actions.len(), 2);
}

#[test]
fn an_uncorrelated_action_is_never_deduplicated() {
    let mut actor = actor();
    let _receiver = register_client(&mut actor);
    let client = ClientKey::new(1);

    actor.apply_to_adapter(client, arm_frame(0));
    actor.apply_to_adapter(client, arm_frame(0));
    assert_eq!(
        actor.adapter.applied_actions.len(),
        2,
        "id 0 carries no identity to deduplicate on"
    );
}

#[test]
fn a_retransmission_with_a_live_intent_still_applies_the_intent() {
    let mut actor = actor();
    let _receiver = register_client(&mut actor);
    let client = ClientKey::new(1);

    actor.apply_to_adapter(client, arm_frame(9));
    // The retransmitted action rides a frame that ALSO carries the live
    // setpoint payload; stripping the answered action must not drop the
    // setpoint.
    let mut frame = motion_control_frame();
    frame.actions = vec![pilotage_protocol::ControlAction::Arm];
    frame.action_ids = vec![9];
    actor.apply_to_adapter(client, frame);
    assert_eq!(actor.adapter.applied_actions.len(), 2);
    assert!(
        actor.adapter.applied_actions[1].is_empty(),
        "the answered action is stripped from the applied frame"
    );
}

#[test]
fn a_disconnect_forgets_the_cache_so_a_new_session_can_reuse_ids() {
    let mut actor = actor();
    let _receiver = register_client(&mut actor);
    let client = ClientKey::new(1);

    actor.apply_to_adapter(client, arm_frame(7));
    actor.action_dedup.forget(client);
    actor.apply_to_adapter(client, arm_frame(7));
    assert_eq!(
        actor.adapter.applied_actions.len(),
        2,
        "ids are per-connection, not global"
    );
}

#[test]
fn a_correlation_id_reused_with_different_content_is_refused() {
    let mut actor = actor();
    let _receiver = register_client(&mut actor);
    let client = ClientKey::new(1);

    actor.apply_to_adapter(client, arm_frame(7));
    // The same id now arrives carrying a DIFFERENT action: never executed,
    // never answered with the cached "accepted" for the original press.
    let mut reused = arm_frame(7);
    reused.actions = vec![pilotage_protocol::ControlAction::Disarm];
    actor.apply_to_adapter(client, reused);
    assert_eq!(
        actor.adapter.applied_actions.len(),
        1,
        "the smuggled action must not reach the adapter"
    );
}

/// Collects the correlation id of every ControlActionResult queued for the
/// client.
fn drain_result_ids(receiver: &mut mpsc::Receiver<ToConnection>) -> Vec<u32> {
    let mut ids = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        let ToConnection::BootstrapMessage(bytes) = message else {
            continue;
        };
        let Ok(envelope) =
            <pilotage_protocol::wire::Envelope as prost::Message>::decode_length_delimited(
                bytes.as_slice(),
            )
        else {
            continue;
        };
        if let Some(pilotage_protocol::wire::envelope::Payload::ControlActionResult(result)) =
            envelope.payload
        {
            ids.push(result.action_id);
        }
    }
    ids
}

#[test]
fn an_early_adapter_rejection_still_answers_every_correlated_action() {
    // The result guarantee: the recording adapter's link-loss latch makes
    // apply_control reject the whole frame BEFORE per-action disposal —
    // the actor must synthesize the correlated rejection itself, or the
    // sender times out on a press the host actually consumed.
    let mut actor = actor();
    let mut receiver = register_client(&mut actor);
    let client = ClientKey::new(1);
    actor
        .adapter
        .latched
        .push(pilotage_protocol::ScopeId::new(MOTION));

    actor.apply_to_adapter(client, arm_frame(3));
    let ids = drain_result_ids(&mut receiver);
    assert_eq!(ids, vec![3], "exactly one correlated result: {ids:?}");
}

#[test]
fn an_evicted_id_stays_stale_and_never_executes_again() {
    // Anti-replay must survive cache eviction: push more than the cache
    // bound of fresh ids, then replay the very first — it must be REFUSED
    // (an answer still arrives) and must NOT reach the adapter.
    let mut actor = actor();
    let mut receiver = register_client(&mut actor);
    let client = ClientKey::new(1);
    for id in 1..=80u32 {
        actor.apply_to_adapter(client, arm_frame(id));
    }
    let executed = actor.adapter.applied_actions.len();
    assert_eq!(executed, 80, "eighty distinct presses executed");
    drain_result_ids(&mut receiver);

    // Id 1 was evicted from the bounded cache long ago.
    actor.apply_to_adapter(client, arm_frame(1));
    assert_eq!(
        actor.adapter.applied_actions.len(),
        executed,
        "the stale id must never execute again"
    );
    let ids = drain_result_ids(&mut receiver);
    assert_eq!(ids, vec![1], "the stale replay is still answered: {ids:?}");
}

#[test]
fn the_watermark_wraps_at_u32_without_stalling_fresh_ids() {
    let mut actor = actor();
    let _receiver = register_client(&mut actor);
    let client = ClientKey::new(1);
    actor.apply_to_adapter(client, arm_frame(u32::MAX - 1));
    actor.apply_to_adapter(client, arm_frame(u32::MAX));
    // The sender's counter wraps past zero (0 is never minted): 1 is a
    // legitimate forward advance, not a stale replay.
    actor.apply_to_adapter(client, arm_frame(1));
    assert_eq!(
        actor.adapter.applied_actions.len(),
        3,
        "the wrap advance executes"
    );
    // ...and the pre-wrap id is now stale.
    actor.apply_to_adapter(client, arm_frame(u32::MAX - 1));
    assert_eq!(actor.adapter.applied_actions.len(), 3);
}

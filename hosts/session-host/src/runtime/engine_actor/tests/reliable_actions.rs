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

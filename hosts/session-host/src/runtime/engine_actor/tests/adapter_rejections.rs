//! Reliable adapter-rejection delivery and transition deduplication.

use super::fixtures::{actor, motion_control_frame, register_client};
use super::*;

fn drain_rejections(
    receiver: &mut mpsc::Receiver<ToConnection>,
) -> Vec<pilotage_protocol::FrameRejected> {
    let mut rejections = Vec::new();
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
        if let Some(pilotage_protocol::wire::envelope::Payload::FrameRejected(rejected)) =
            envelope.payload
        {
            rejections.push(
                pilotage_protocol::FrameRejected::try_from(rejected)
                    .expect("valid frame rejection"),
            );
        }
    }
    rejections
}

#[test]
fn an_uplink_idle_rejection_reaches_the_sender_once_per_transition() {
    let mut actor = actor();
    let mut receiver = register_client(&mut actor);
    let client = ClientKey::new(1);
    actor.adapter.reject_control = Some(RejectReason::UplinkIdle);

    actor.apply_to_adapter(client, motion_control_frame());
    actor.apply_to_adapter(client, motion_control_frame());
    let notices = drain_rejections(&mut receiver);
    assert_eq!(
        notices.len(),
        1,
        "a 30 Hz refusal is transition-deduplicated"
    );
    assert_eq!(notices[0].scope, ScopeId::new(MOTION));
    assert_eq!(notices[0].current_generation, Generation::new(1));
    assert_eq!(
        notices[0].reason,
        pilotage_protocol::FrameRejectionReason::UplinkIdle
    );

    actor.adapter.reject_control = None;
    actor.apply_to_adapter(client, motion_control_frame());
    actor.adapter.reject_control = Some(RejectReason::UplinkIdle);
    let mut next = motion_control_frame();
    next.sequence = SequenceNum::new(2);
    actor.apply_to_adapter(client, next);
    assert_eq!(
        drain_rejections(&mut receiver).len(),
        1,
        "an enacted frame arms a fresh rejection transition"
    );
}

use pilotage_adapter_api::{Disposition, LinkLossPolicy};
use pilotage_protocol::Generation;

use super::{Harness, Ordering, commands, frame, observed_poses, packets};
use crate::adapter::DjiFault;

#[cfg(test)]
fn moving_with_keyframe(write_limit: usize) -> Harness {
    let mut harness = Harness::mobile();
    for (index, pose) in observed_poses().iter().enumerate() {
        harness
            .feed(&crate::logiclink::encode_control(&packets(pose)[0]))
            .unwrap();
        harness.step(index as u64 * 10);
    }
    assert_eq!(
        harness.apply(frame(0.01), 25).disposition,
        Disposition::Transformed
    );
    harness.step(25);
    let video = harness.adapter.subscribe_encoded_video().unwrap();
    harness.set_write_limit(write_limit).unwrap();
    video.keyframe_request.store(true, Ordering::Release);
    harness.step(30);
    harness
}

#[test]
fn neutral_hold_preempts_between_keyframe_packets_and_preserves_recovery_interval() {
    let mut harness = moving_with_keyframe(usize::MAX);
    assert_eq!(commands(&harness).len(), 3);
    assert_eq!(
        harness.apply(frame(0.0), 35).disposition,
        Disposition::Transformed
    );
    harness.step(35);
    let written = commands(&harness);
    assert_eq!(written.len(), 4);
    assert_eq!(
        written
            .iter()
            .map(|packet| (packet.command_set(), packet.command_id(), packet.sequence()))
            .collect::<Vec<_>>(),
        [(0, 1, 1), (4, 20, 2), (1, 1, 3), (4, 20, 6)]
    );
    harness.step(1029);
    assert_eq!(commands(&harness).len(), 4);
    harness.step(1030);
    let resumed = commands(&harness);
    assert_eq!(resumed.len(), 5);
    assert_eq!((resumed[4].command_id(), resumed[4].sequence()), (1, 7));
    assert!(harness.adapter.status_cached().fault.is_none());
}

#[test]
fn link_loss_hold_preempts_a_complete_keyframe_envelope() {
    let mut harness = moving_with_keyframe(usize::MAX);
    harness
        .link_loss(Some(LinkLossPolicy::Neutralize), 35)
        .expect("finite hold reached the transport");
    let written = commands(&harness);
    assert_eq!(written.len(), 4);
    assert_eq!(written[3].command_set(), 4);
    assert_eq!(written[3].command_id(), 20);
    assert_eq!(written[3].sequence(), 6);
    assert!(harness.adapter.status_cached().fault.is_none());
}

#[test]
fn new_authority_does_not_wait_for_the_cancelled_keyframe_suffix() {
    let mut harness = moving_with_keyframe(usize::MAX);
    let mut request = frame(0.01);
    request.generation = Generation::new(2);
    assert_eq!(
        harness.apply(request, 35).disposition,
        Disposition::Transformed
    );
    harness.step(35);
    let written = commands(&harness);
    assert_eq!(written.len(), 4);
    assert_eq!(written[3].command_id(), 20);
    assert_eq!(written[3].sequence(), 6);
    assert!(matches!(
        harness.apply(frame(0.01), 40).disposition,
        Disposition::Rejected(_)
    ));
}

#[test]
fn link_loss_during_a_partial_keyframe_closes_without_writing_a_hold_or_suffix() {
    let mut harness = moving_with_keyframe(4);
    let prefix = harness.written().unwrap();
    assert!(
        harness
            .link_loss(Some(LinkLossPolicy::Neutralize), 35)
            .is_err()
    );
    assert!(matches!(
        harness.adapter.status_cached().fault,
        Some(DjiFault::PartialCommand)
    ));
    harness.set_write_limit(usize::MAX).unwrap();
    harness.step(80);
    harness.step(1030);
    assert_eq!(harness.written().unwrap(), prefix);
    assert!(matches!(
        harness.apply(frame(0.0), 1030).disposition,
        Disposition::Rejected(_)
    ));
}

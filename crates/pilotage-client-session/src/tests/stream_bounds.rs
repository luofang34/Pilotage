//! Video-record framing bounds: an impossible length claim fails ONE
//! stream closed with its memory returned, and no other stream is
//! touched. Unbounded, the reassembly buffer grows at line rate until
//! the process dies inside a gigabyte allocation.

#![allow(clippy::expect_used, clippy::panic)]

use super::{admit, engine};
use crate::{ClientAction, ModuleEvent, StreamId, TransportEvent};

fn video_open(engine: &mut crate::ClientEngine, id: u64) {
    engine.handle(TransportEvent::UniStreamOpened(StreamId(id)), 0);
    engine.handle(
        TransportEvent::UniStreamReceived(StreamId(id), vec![0x04]),
        0,
    );
}

#[test]
fn an_impossible_record_claim_fails_the_stream_closed() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);
    video_open(&mut engine, 5);

    // A healthy record drains normally.
    let mut healthy = 4u32.to_be_bytes().to_vec();
    healthy.extend_from_slice(&[1, 2, 3, 4]);
    let actions = engine.handle(TransportEvent::UniStreamReceived(StreamId(5), healthy), 0);
    assert!(actions.iter().any(|action| matches!(
        action,
        ClientAction::Emit(ModuleEvent::VideoFrame(body)) if body == &[1, 2, 3, 4]
    )));

    // A gigabyte claim is framing garbage: the stream fails closed and
    // says so, and its buffer is given back.
    let giant = u32::MAX.to_be_bytes().to_vec();
    let actions = engine.handle(TransportEvent::UniStreamReceived(StreamId(5), giant), 0);
    assert!(actions.iter().any(|action| matches!(
        action,
        ClientAction::Emit(ModuleEvent::VideoStreamCorrupt { claimed_bytes })
            if *claimed_bytes == u32::MAX as usize
    )));
    assert_eq!(engine.stream_pending_bytes(), 0);

    // Everything that follows on the corrupt stream is discarded...
    let mut late = 4u32.to_be_bytes().to_vec();
    late.extend_from_slice(&[9, 9, 9, 9]);
    let actions = engine.handle(TransportEvent::UniStreamReceived(StreamId(5), late), 0);
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, ClientAction::Emit(ModuleEvent::VideoFrame(_)))),
        "a corrupt stream must not speak again"
    );

    // ...while a sibling video stream is untouched.
    video_open(&mut engine, 6);
    let mut sibling = 2u32.to_be_bytes().to_vec();
    sibling.extend_from_slice(&[7, 7]);
    let actions = engine.handle(TransportEvent::UniStreamReceived(StreamId(6), sibling), 0);
    assert!(actions.iter().any(|action| matches!(
        action,
        ClientAction::Emit(ModuleEvent::VideoFrame(body)) if body == &[7, 7]
    )));
}

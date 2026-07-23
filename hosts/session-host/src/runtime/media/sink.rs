//! Per-source handoff state and bounded writer-respawn transitions.

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

use super::EncodedFrame;

pub(super) type FrameTx = mpsc::Sender<EncodedFrame>;

pub(super) const MAX_WRITER_RESPAWNS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SinkAction {
    Respawn,
    Retire,
}

pub(super) fn on_writer_exit(writer_exits: u32) -> SinkAction {
    if writer_exits < MAX_WRITER_RESPAWNS {
        SinkAction::Respawn
    } else {
        SinkAction::Retire
    }
}

pub(super) enum ClientSink {
    Live {
        frames: FrameTx,
        dropped: u64,
        writer_exits: u32,
    },
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SinkDelivery {
    Delivered,
    DroppedFull(u64),
    Respawned(u32),
    Retired(u32),
    Skipped,
}

pub(super) fn deliver_to_sink(
    sink: &mut ClientSink,
    encoded: &EncodedFrame,
    respawn: impl FnOnce(u32) -> ClientSink,
) -> SinkDelivery {
    let ClientSink::Live {
        frames,
        dropped,
        writer_exits,
    } = sink
    else {
        return SinkDelivery::Skipped;
    };
    match frames.try_send(encoded.clone()) {
        Ok(()) => SinkDelivery::Delivered,
        Err(TrySendError::Full(_)) => {
            *dropped = dropped.wrapping_add(1);
            SinkDelivery::DroppedFull(*dropped)
        }
        Err(TrySendError::Closed(_)) => {
            let exits = *writer_exits;
            match on_writer_exit(exits) {
                SinkAction::Respawn => {
                    let exits = exits.wrapping_add(1);
                    *sink = respawn(exits);
                    if let ClientSink::Live { frames, .. } = sink {
                        frames.try_send(encoded.clone()).ok();
                    }
                    SinkDelivery::Respawned(exits)
                }
                SinkAction::Retire => {
                    *sink = ClientSink::Retired;
                    SinkDelivery::Retired(exits)
                }
            }
        }
    }
}

pub(super) fn fully_retired(sources: &std::collections::BTreeMap<u8, ClientSink>) -> bool {
    !sources.is_empty()
        && sources
            .values()
            .all(|sink| matches!(sink, ClientSink::Retired))
}

//! The frame tap: the media path keeps every picture; the lane gets a copy
//! of one picture per period, when it can take it.

use std::sync::Arc;
use std::time::Duration;

use pilotage_adapter_api::RawVideoFrame;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::LaneInbound;

/// Pictures the tap holds for the media path.
const TAP_QUEUE: usize = 4;
/// The lane samples at most one picture per period.
const SAMPLE_PERIOD: Duration = Duration::from_millis(1_000);

/// Forwards `frames` unchanged and sends the lane one picture per period.
/// Without a lane the frames pass through untouched.
pub(crate) fn tap(
    frames: mpsc::Receiver<RawVideoFrame>,
    lane: Option<mpsc::Sender<LaneInbound>>,
) -> mpsc::Receiver<RawVideoFrame> {
    let Some(lane) = lane else {
        return frames;
    };
    let (sender, receiver) = mpsc::channel(TAP_QUEUE);
    tokio::spawn(forward(frames, sender, lane));
    receiver
}

async fn forward(
    mut frames: mpsc::Receiver<RawVideoFrame>,
    media: mpsc::Sender<RawVideoFrame>,
    lane: mpsc::Sender<LaneInbound>,
) {
    let mut last: Option<Instant> = None;
    while let Some(frame) = frames.recv().await {
        if last.is_none_or(|at| at.elapsed() >= SAMPLE_PERIOD) {
            last = Some(Instant::now());
            // A busy lane skips the sample; the media path never waits.
            lane.try_send(LaneInbound::Frame(Arc::new(frame.clone())))
                .ok();
        }
        if media.send(frame).await.is_err() {
            break;
        }
    }
    tracing::debug!(task = "frame-tap", "task exited");
}

//! The producer side of the `pilotage.bridge.v1` link to the session host.
//!
//! The producer dials the host's sim camera port, folds the host's poses and
//! camera commands into latest-value state the render thread reads, and
//! writes the frames the render thread hands it. A frame is never queued
//! behind a newer one: a full queue drops the frame and counts it, because
//! a late frame of a moving camera is a wrong frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pilotage_sim_video::read_envelope;
use pilotage_sim_video::wire::{
    BridgeCameraCommand, BridgeEnvelope, BridgeFrame, BridgeViewPose, bridge_envelope::Payload,
};
use prost::Message;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

/// How long the producer waits before dialing a host that is not there.
const REDIAL_INTERVAL: Duration = Duration::from_secs(1);
/// Frames waiting for the socket; one in flight and one ready.
const FRAME_QUEUE_DEPTH: usize = 2;

/// A pose and when the producer received it, on its own monotonic clock.
#[derive(Debug, Clone, Copy)]
pub struct ReceivedPose {
    /// The pose.
    pub pose: BridgeViewPose,
    /// When it arrived.
    pub received_at: std::time::Instant,
}

/// The render thread's view of the link.
pub struct LinkEnds {
    /// The newest pose from the host.
    pub poses: watch::Receiver<Option<ReceivedPose>>,
    /// The newest camera command from the host.
    pub commands: watch::Receiver<Option<BridgeCameraCommand>>,
    /// Where finished frames go.
    pub frames: mpsc::Sender<BridgeFrame>,
    /// Frames dropped because the socket was behind.
    pub dropped: Arc<AtomicU64>,
}

/// Starts the link task on the current runtime. It dials `127.0.0.1:port`
/// until the host answers and dials again whenever the host goes away.
#[must_use]
pub fn spawn(port: u16) -> (LinkEnds, tokio::task::JoinHandle<()>) {
    let (pose_tx, poses) = watch::channel(None);
    let (command_tx, commands) = watch::channel(None);
    let (frames, frame_rx) = mpsc::channel(FRAME_QUEUE_DEPTH);
    let task = tokio::spawn(run(port, pose_tx, command_tx, frame_rx));
    let ends = LinkEnds {
        poses,
        commands,
        frames,
        dropped: Arc::new(AtomicU64::new(0)),
    };
    (ends, task)
}

async fn run(
    port: u16,
    pose_tx: watch::Sender<Option<ReceivedPose>>,
    command_tx: watch::Sender<Option<BridgeCameraCommand>>,
    mut frames: mpsc::Receiver<BridgeFrame>,
) {
    let mut interval = tokio::time::interval(REDIAL_INTERVAL);
    loop {
        interval.tick().await;
        let stream = match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::debug!(port, %error, "host sim camera port not answering");
                continue;
            }
        };
        if let Err(error) = stream.set_nodelay(true) {
            warn!(%error, "could not disable Nagle on the host link");
        }
        info!(port, "attached to the session host");
        // A new attachment renders nothing until the host states a pose.
        pose_tx.send_replace(None);
        let (mut reader, mut writer) = stream.into_split();
        let inbound = async {
            loop {
                match read_envelope(&mut reader).await {
                    Ok(Some(envelope)) => fold(envelope, &pose_tx, &command_tx),
                    Ok(None) => return "the host closed the link".to_owned(),
                    Err(error) => return format!("reading from the host failed: {error}"),
                }
            }
        };
        let outbound = async {
            while let Some(frame) = frames.recv().await {
                let envelope = BridgeEnvelope {
                    payload: Some(Payload::Frame(frame)),
                };
                let bytes = envelope.encode_length_delimited_to_vec();
                if let Err(error) = writer.write_all(&bytes).await {
                    return format!("writing to the host failed: {error}");
                }
            }
            "the render thread stopped".to_owned()
        };
        let reason = tokio::select! {
            reason = inbound => reason,
            reason = outbound => reason,
        };
        info!(%reason, "detached from the session host");
        if frames.is_closed() {
            return;
        }
    }
}

fn fold(
    envelope: BridgeEnvelope,
    pose_tx: &watch::Sender<Option<ReceivedPose>>,
    command_tx: &watch::Sender<Option<BridgeCameraCommand>>,
) {
    match envelope.payload {
        Some(Payload::ViewPose(pose)) => {
            pose_tx.send_replace(Some(ReceivedPose {
                pose,
                received_at: std::time::Instant::now(),
            }));
        }
        Some(Payload::CameraCommand(command)) => {
            command_tx.send_replace(Some(command));
        }
        // Odometry, fixes, frames, and controls travel the other way or
        // belong to other producers.
        _ => {}
    }
}

impl LinkEnds {
    /// Hands a frame to the link, or drops and counts it when the socket is
    /// behind. Returns `false` once the link task has ended.
    pub fn offer(&self, frame: BridgeFrame) -> bool {
        match self.frames.try_send(frame) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}

#[cfg(test)]
mod tests;

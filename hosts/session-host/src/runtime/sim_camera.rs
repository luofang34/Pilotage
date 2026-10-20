//! The simulator camera attachment: a camera producer that renders the
//! vehicle's views from a world model, for any adapter.
//!
//! The attachment reads the host's own telemetry broadcast as an
//! in-process client of the engine actor, reduces each simulator-truth
//! sample to a [`BridgeViewPose`](pilotage_sim_video::wire::BridgeViewPose),
//! and sends the newest pose to the producer that dialed its loopback
//! port. Frames come back stamped with the pose's truth clock reading, so
//! the capture clock IS the truth clock and the frames declare the
//! identity mapping onto it (ADR-0020). A vehicle without simulator truth
//! gets no pose, and its producer renders nothing.
//!
//! The producer outlives a session the way a simulator does: the listener
//! stays bound for the host's lifetime and each new connection is a new
//! capture incarnation.

mod pose;

use std::collections::BTreeMap;

use pilotage_adapter_api::{
    CaptureClockMapping, FrameStamper, MeasurementClock, RawVideoFrame, SourceIncarnation,
};
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use pilotage_sim_video::BridgeClient;
use prost::Message;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use super::connection::ToConnection;
use super::engine_actor::ToEngine;
use super::registry::OUTBOUND_QUEUE_CAPACITY;
use crate::error::HostError;
use pose::{TruthPose, truth_pose};

/// The attachment's driver-assigned client key, from the same
/// collision-free end of the key space as the mission principal's.
const SIM_CAMERA_CLIENT: ClientKey = ClientKey::new(u64::MAX - 2);

/// Bounded frame-channel depth: small, so a slow media consumer drops
/// stale frames instead of growing latency.
const FRAME_CHANNEL_DEPTH: usize = 4;

/// Binds the producer port on loopback, so a port another process holds
/// fails host startup instead of leaving a camera that never attaches.
///
/// # Errors
///
/// [`HostError::SimCameraBind`] when the port cannot be bound.
pub(super) async fn bind(port: u16) -> Result<TcpListener, HostError> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|source| HostError::SimCameraBind { port, source })
}

/// Starts the attachment on `listener`: registers with the engine actor,
/// and returns the stamped-frame receiver for the media task and the
/// task's handle. The task ends when the engine actor does.
pub(super) fn spawn(
    listener: TcpListener,
    engine_tx: &mpsc::Sender<ToEngine>,
) -> (mpsc::Receiver<RawVideoFrame>, JoinHandle<()>) {
    let (frames_tx, frames_rx) = mpsc::channel(FRAME_CHANNEL_DEPTH);
    let engine_tx = engine_tx.clone();
    let task = tokio::spawn(async move {
        run(listener, engine_tx, frames_tx).await;
        tracing::debug!(task = "sim-camera", "task exited");
    });
    (frames_rx, task)
}

async fn run(
    listener: TcpListener,
    engine_tx: mpsc::Sender<ToEngine>,
    frames_tx: mpsc::Sender<RawVideoFrame>,
) {
    let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
    let registered = engine_tx
        .send(ToEngine::ClientConnected {
            client: SIM_CAMERA_CLIENT,
            sender: outbound_tx,
        })
        .await;
    // Holding the engine's command channel would keep the actor alive at
    // shutdown; the registration is the only message this task sends it.
    drop(engine_tx);
    if registered.is_err() {
        return;
    }
    let (pose_tx, mut pose_rx) = watch::channel(None);
    let tap = tokio::spawn(tap_truth(outbound_rx, pose_tx));
    let mut session = 0_u64;
    loop {
        let bridge = tokio::select! {
            accepted = BridgeClient::accept_from(&listener, FRAME_CHANNEL_DEPTH) => accepted,
            // The tap ends with the engine actor, and so does this task.
            () = closed(&mut pose_rx) => break,
        };
        match bridge {
            Ok(bridge) => {
                session = session.wrapping_add(1);
                info!(session, "sim camera producer attached");
                if !serve(bridge, &mut pose_rx, &frames_tx, incarnation(session)).await {
                    break;
                }
                info!(session, "sim camera producer detached");
            }
            Err(error) => warn!(%error, "sim camera producer accept failed"),
        }
    }
    tap.abort();
}

/// Resolves once the truth tap has ended.
async fn closed(pose_rx: &mut watch::Receiver<Option<TruthPose>>) {
    while pose_rx.changed().await.is_ok() {}
}

/// Folds the engine's telemetry broadcast into the newest truth pose.
async fn tap_truth(
    mut outbound: mpsc::Receiver<ToConnection>,
    pose_tx: watch::Sender<Option<TruthPose>>,
) {
    while let Some(message) = outbound.recv().await {
        let bytes = match message {
            ToConnection::Datagram { bytes, .. } => bytes,
            ToConnection::Close => {
                warn!("the engine closed the sim camera's telemetry; no more poses");
                return;
            }
            ToConnection::BootstrapMessage { .. } | ToConnection::AuthorityMessage(_) => continue,
        };
        let Ok(envelope) = wire::Envelope::decode(bytes.as_slice()) else {
            continue;
        };
        let Some(wire::envelope::Payload::TelemetrySample(sample)) = envelope.payload else {
            continue;
        };
        let Some(pose) = truth_pose(&sample) else {
            continue;
        };
        // A truth group is re-broadcast unchanged until a new observation
        // arrives; only a new one is a new pose.
        let fresh = pose_tx
            .borrow()
            .is_none_or(|previous| previous.pose.sim_time_ns != pose.pose.sim_time_ns);
        if fresh {
            pose_tx.send_replace(Some(pose));
        }
    }
}

/// Serves one producer connection: sends every new pose, stamps every
/// frame. Returns `false` when the host is shutting down, `true` when the
/// producer left.
async fn serve(
    mut bridge: BridgeClient,
    pose_rx: &mut watch::Receiver<Option<TruthPose>>,
    frames_tx: &mpsc::Sender<RawVideoFrame>,
    incarnation: SourceIncarnation,
) -> bool {
    let Some(mut frames) = bridge.take_frame_rx() else {
        return true;
    };
    // A producer that attaches mid-flight renders the current pose at once.
    pose_rx.mark_changed();
    let mut stamper: Option<(MeasurementClock, FrameStamper)> = None;
    loop {
        tokio::select! {
            changed = pose_rx.changed() => {
                if changed.is_err() {
                    return false;
                }
                let pose = *pose_rx.borrow_and_update();
                if let Some(pose) = pose
                    && !bridge.try_send_view_pose(pose.pose)
                {
                    return true;
                }
            }
            frame = frames.recv() => {
                let Some(frame) = frame else {
                    return true;
                };
                // Frames are captured on the clock of the truth they were
                // rendered from; before any truth there is no clock to name.
                let Some(clock) = pose_rx.borrow().map(|pose| pose.clock) else {
                    continue;
                };
                let stamper = match &mut stamper {
                    Some((current, stamper)) if *current == clock => stamper,
                    slot => &mut slot.insert((clock, frame_stamper(incarnation, clock))).1,
                };
                if frames_tx.send(stamper.stamp(frame.into())).await.is_err() {
                    return false;
                }
            }
        }
    }
}

fn frame_stamper(incarnation: SourceIncarnation, clock: MeasurementClock) -> FrameStamper {
    FrameStamper::new(
        incarnation,
        clock,
        CaptureClockMapping::identity(clock),
        BTreeMap::new(),
    )
}

/// A capture incarnation distinct across producer sessions and host runs.
fn incarnation(session: u64) -> SourceIncarnation {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&now.as_nanos().to_le_bytes()[..8]);
    bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    bytes[12..].copy_from_slice(&session.to_le_bytes()[..4]);
    SourceIncarnation::new(bytes)
}

#[cfg(test)]
mod tests;

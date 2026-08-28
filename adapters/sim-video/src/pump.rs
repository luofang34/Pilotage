//! The bridge connection's two background tasks: the reader that folds
//! inbound envelopes into the latest-value caches and the bounded frame
//! channel, and the writer that emits the latest control and camera
//! commands.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use prost::Message;
use tokio::io::{AsyncWriteExt, WriteHalf};
use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

use crate::bridge_client::{LatestBridgeState, ReaderHealth};
use crate::framing::read_envelope;
use crate::wire::{
    BridgeCameraCommand, BridgeControl, BridgeEnvelope, BridgeFrame, bridge_envelope,
};

/// Reads length-delimited envelopes until EOF or error: odometry updates the
/// shared latest-value state; frames go to the bounded channel, counting drops.
///
/// On every exit path it publishes an [`ReaderHealth::Ended`] status carrying
/// the reason, so `reader_health` can surface the liveness loss instead of
/// letting `sample_telemetry` return a frozen odometry cache forever.
pub(crate) async fn reader_loop(
    mut read_half: tokio::io::ReadHalf<tokio::net::TcpStream>,
    state_tx: watch::Sender<LatestBridgeState>,
    frame_tx: mpsc::Sender<BridgeFrame>,
    reader_health_tx: watch::Sender<ReaderHealth>,
    dropped_frames: Arc<AtomicU64>,
) {
    let reason = loop {
        match read_envelope(&mut read_half).await {
            Ok(Some(envelope)) => {
                handle_envelope(envelope, &state_tx, &frame_tx, &dropped_frames);
            }
            Ok(None) => {
                debug!("sidecar bridge closed the connection");
                break "sidecar bridge closed the connection".to_owned();
            }
            Err(err) => {
                warn!(error = %err, "sidecar bridge read failed; stopping reader");
                break format!("sidecar bridge read failed: {err}");
            }
        }
    };
    // A closed receiver is impossible while the client is alive: the client
    // owns the sole `reader_health_rx`, and dropping it aborts this task. So
    // this publish is the client's one liveness signal for a self-terminated
    // reader.
    reader_health_tx.send_replace(ReaderHealth::Ended(reason));
}

fn handle_envelope(
    envelope: BridgeEnvelope,
    state_tx: &watch::Sender<LatestBridgeState>,
    frame_tx: &mpsc::Sender<BridgeFrame>,
    dropped_frames: &Arc<AtomicU64>,
) {
    match envelope.payload {
        Some(bridge_envelope::Payload::Odometry(odometry)) => {
            // A closed receiver is impossible here: the client owns the sole
            // `state_rx`, so `send` only fails after the client is dropped,
            // which also aborts this task.
            let navsat = state_tx.borrow().navsat;
            state_tx.send_replace(LatestBridgeState {
                odometry: Some(odometry),
                navsat,
            });
        }
        Some(bridge_envelope::Payload::Navsat(navsat)) => {
            let odometry = state_tx.borrow().odometry;
            state_tx.send_replace(LatestBridgeState {
                odometry,
                navsat: Some(navsat),
            });
        }
        Some(bridge_envelope::Payload::Frame(frame)) => {
            if let Err(mpsc::error::TrySendError::Full(_)) = frame_tx.try_send(frame) {
                dropped_frames.fetch_add(1, Ordering::Relaxed);
            }
        }
        // The host never receives control envelopes; ignore anything else.
        _ => {}
    }
}

/// Writes the latest published control as a length-delimited envelope whenever
/// it changes. A slow socket coalesces intervening updates to the newest value
/// (latest-valid-value, ADR-0009). Exits on channel close (client dropped) or a
/// socket write error.
pub(crate) async fn writer_loop(
    mut write_half: WriteHalf<tokio::net::TcpStream>,
    mut control_rx: watch::Receiver<Option<BridgeControl>>,
    mut camera_rx: watch::Receiver<Option<BridgeCameraCommand>>,
) {
    loop {
        let payload = tokio::select! {
            changed = control_rx.changed() => {
                if changed.is_err() {
                    return;
                }
                (*control_rx.borrow_and_update()).map(bridge_envelope::Payload::Control)
            }
            changed = camera_rx.changed() => {
                if changed.is_err() {
                    return;
                }
                (*camera_rx.borrow_and_update()).map(bridge_envelope::Payload::CameraCommand)
            }
        };
        let Some(payload) = payload else {
            continue;
        };
        let envelope = BridgeEnvelope {
            payload: Some(payload),
        };
        let bytes = envelope.encode_length_delimited_to_vec();
        if let Err(err) = write_half.write_all(&bytes).await {
            warn!(error = %err, "sidecar bridge write failed; stopping writer");
            return;
        }
    }
}

//! The camera producer path. Which producer feeds the frames is a
//! deployment property, selected by `PILOTAGE_AVIATE_CAMERA`:
//!
//! - unset or `on`: Pilotage's C++ gz-transport sidecar delivers the
//!   flight world's `/camera` and `/chase_camera` frames.
//! - `xplane-plugin`: the in-simulator Pilotage camera plugin dials this
//!   process and delivers the vehicle camera view (FPV or gimbal
//!   payload), and accepts pointing and zoom commands back.
//! - `off`: no video.
//!
//! The frames are captured on the producer's own clock, but Aviate's
//! flight state is estimated on the flight controller's vehicle-boot
//! clock. No correlation between those two clocks is available, so every
//! frame is stamped with an unavailable clock mapping (ADR-0020): a
//! consumer must gate conformal overlay off rather than draw against a
//! state it cannot align to the image. The capture identity itself
//! (source, epoch, sequence, capture time) is still preserved honestly.

use std::collections::BTreeMap;

use pilotage_adapter_api::{CalibrationId, FrameStamper, MeasurementClock};

use crate::incarnation::{IncarnationProvider, OsIncarnationProvider};

use super::pointing::XPLANE_CAMERA_PORT;

/// Bounded frame-channel depth: small, so a slow media consumer drops
/// stale frames instead of growing latency.
const FRAME_CHANNEL_DEPTH: usize = 4;

/// How this session sources camera frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CameraMode {
    /// The gz-transport sidecar (FPV + chase), spawned by this process.
    Gazebo,
    /// The in-simulator camera plugin, which dials this process and
    /// takes pointing and zoom commands.
    XPlanePlugin,
    /// No video.
    Off,
}

/// Resolves `PILOTAGE_AVIATE_CAMERA` — an unknown value degrades to no
/// video with a warning rather than inventing a producer.
pub(crate) fn camera_mode() -> CameraMode {
    match std::env::var("PILOTAGE_AVIATE_CAMERA").as_deref() {
        Err(_) | Ok("on") => CameraMode::Gazebo,
        Ok("xplane-plugin") => CameraMode::XPlanePlugin,
        Ok("off") => CameraMode::Off,
        Ok(other) => {
            tracing::warn!(value = other, "unknown PILOTAGE_AVIATE_CAMERA; no video");
            CameraMode::Off
        }
    }
}

/// The calibration binding for a detented-zoom producer.
///
/// A detent IS a distinct camera model, so the honest binding is
/// per-FRAME (the detent the picture was captured at), not per-camera.
/// The frame stamper binds calibrations per CAMERA ID, and the producer's
/// frames do not yet carry the detent they were captured at, so binding a
/// fixed detent here would stamp a zoomed picture with a camera model it
/// was not captured with — exactly the silent assumption ADR-0021
/// forbids. The map is therefore EMPTY: frames stamp
/// `CalibrationId::NONE` and a conformal consumer keeps its gate closed
/// until the producer reports its detent per frame.
fn detent_calibrations() -> BTreeMap<u32, CalibrationId> {
    BTreeMap::new()
}

/// Attaches the session's camera producer, degrading to no-video when it
/// can't (`PILOTAGE_AVIATE_CAMERA=off` disables the attempt).
#[allow(clippy::type_complexity)]
pub(crate) async fn spawn_camera_bridge() -> (
    Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>>,
    Option<pilotage_sim_video::BridgeClient>,
    Option<tokio::task::JoinHandle<()>>,
) {
    let mode = camera_mode();
    if mode == CameraMode::Off {
        return (None, None, None);
    }
    let incarnation = match OsIncarnationProvider.next_incarnation_blocking() {
        Ok(incarnation) => incarnation,
        Err(error) => {
            tracing::warn!(%error, "no capture incarnation available; no video");
            return (None, None, None);
        }
    };
    let (attached, clock, calibrations) = match mode {
        CameraMode::Gazebo => {
            let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(std::path::Path::parent)
                .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
            let bin = workspace_root.join("adapters/gazebo/bridge/build/pilotage-gz-bridge");
            let config = pilotage_sim_video::BridgeConfig::new("x500", bin);
            (
                pilotage_sim_video::BridgeClient::spawn_and_connect(config).await,
                MeasurementClock::Simulation,
                // The gz rig publishes no calibration.
                BTreeMap::new(),
            )
        }
        CameraMode::XPlanePlugin => {
            tracing::info!(
                port = XPLANE_CAMERA_PORT,
                "waiting for the in-simulator camera plugin"
            );
            (
                pilotage_sim_video::BridgeClient::accept_producer(
                    XPLANE_CAMERA_PORT,
                    FRAME_CHANNEL_DEPTH,
                )
                .await,
                // A simulator window has no clock a consumer can relate
                // to the flight state.
                MeasurementClock::HostMonotonic,
                detent_calibrations(),
            )
        }
        CameraMode::Off => unreachable!("the off mode returned above"),
    };
    match attached {
        Ok(mut bridge) => {
            let (tx, rx) = tokio::sync::mpsc::channel(FRAME_CHANNEL_DEPTH);
            let mut stamper = FrameStamper::new(
                incarnation,
                clock,
                pilotage_adapter_api::CaptureClockMapping::Unavailable,
                calibrations,
            );
            let forwarder = bridge.take_frame_rx().map(|mut bridge_rx| {
                tokio::spawn(async move {
                    while let Some(frame) = bridge_rx.recv().await {
                        if tx.send(stamper.stamp(frame.into())).await.is_err() {
                            return;
                        }
                    }
                })
            });
            tracing::info!(?mode, "Aviate camera producer up");
            (Some(rx), Some(bridge), forwarder)
        }
        Err(error) => {
            tracing::warn!(%error, "camera producer unavailable; no video");
            (None, None, None)
        }
    }
}

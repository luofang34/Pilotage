//! The gz camera sidecar path: Pilotage's own C++ gz-transport bridge
//! delivers the flight world's `/camera` and `/chase_camera` frames.
//!
//! The frames are captured on the sidecar's simulation clock, but Aviate's
//! flight state is estimated on the flight controller's vehicle-boot clock.
//! No correlation between those two clocks is available, so every frame is
//! stamped with an unavailable clock mapping (ADR-0020): a consumer must gate
//! conformal overlay off rather than draw against a state it cannot align to
//! the image. The capture identity itself (source, epoch, sequence, sim
//! capture time) is still preserved honestly.

use pilotage_adapter_gazebo::FrameStamper;

use crate::incarnation::{IncarnationProvider, OsIncarnationProvider};

/// Spawns the gz camera sidecar for the flight world's `/camera` and
/// `/chase_camera` topics, degrading to no-video when it can't
/// (`PILOTAGE_AVIATE_CAMERA=off` disables the attempt).
#[allow(clippy::type_complexity)]
pub(crate) async fn spawn_camera_bridge() -> (
    Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>>,
    Option<pilotage_adapter_gazebo::BridgeClient>,
    Option<tokio::task::JoinHandle<()>>,
) {
    if std::env::var("PILOTAGE_AVIATE_CAMERA").as_deref() == Ok("off") {
        return (None, None, None);
    }
    let incarnation = match OsIncarnationProvider.next_incarnation_blocking() {
        Ok(incarnation) => incarnation,
        Err(error) => {
            tracing::warn!(%error, "no capture incarnation available; no video");
            return (None, None, None);
        }
    };
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    let bin = workspace_root.join("adapters/gazebo/bridge/build/pilotage-gz-bridge");
    let config = pilotage_adapter_gazebo::BridgeConfig::new("x500", bin);
    match pilotage_adapter_gazebo::BridgeClient::spawn_and_connect(config).await {
        Ok(mut bridge) => {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            // Aviate has no correlation between the sim capture clock and the
            // flight controller's clock, so the mapping is unavailable; it also
            // publishes no camera calibration.
            let mut stamper = FrameStamper::new(
                incarnation,
                pilotage_adapter_api::CaptureClockMapping::Unavailable,
                std::collections::BTreeMap::new(),
            );
            let forwarder = bridge.take_frame_rx().map(|mut bridge_rx| {
                tokio::spawn(async move {
                    while let Some(frame) = bridge_rx.recv().await {
                        if tx.send(stamper.stamp(frame)).await.is_err() {
                            return;
                        }
                    }
                })
            });
            tracing::info!("Aviate camera sidecar up (FPV + chase)");
            (Some(rx), Some(bridge), forwarder)
        }
        Err(error) => {
            tracing::warn!(%error, "camera sidecar unavailable; no video");
            (None, None, None)
        }
    }
}

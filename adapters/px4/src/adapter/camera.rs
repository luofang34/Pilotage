//! The gz camera sidecar path for the px4-gz world: Pilotage's C++
//! gz-transport bridge delivers the flight-deck rig's `/camera` and
//! `/chase_camera` frames.
//!
//! Frames are captured on the sidecar's simulation clock while PX4's
//! flight state runs on its own boot clock; no correlation between the
//! two is available, so every frame carries an unavailable clock
//! mapping (ADR-0020) — a consumer must gate conformal overlay off
//! rather than draw against a state it cannot align to the image.

use pilotage_adapter_api::SourceIncarnation;
use pilotage_adapter_gazebo::FrameStamper;

/// Spawns the gz camera sidecar for the rig's `/camera` and
/// `/chase_camera` topics, degrading to no-video when it can't
/// (`PILOTAGE_PX4_CAMERA=off` disables the attempt).
#[allow(clippy::type_complexity)]
pub(crate) async fn spawn_camera_bridge() -> (
    Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>>,
    Option<pilotage_adapter_gazebo::BridgeClient>,
    Option<tokio::task::JoinHandle<()>>,
) {
    if std::env::var("PILOTAGE_PX4_CAMERA").as_deref() == Ok("off") {
        return (None, None, None);
    }
    let incarnation = SourceIncarnation::new(super::rand_incarnation());
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    let bin = workspace_root.join("adapters/gazebo/bridge/build/pilotage-gz-bridge");
    let config = pilotage_adapter_gazebo::BridgeConfig::new("x500", bin);
    match pilotage_adapter_gazebo::BridgeClient::spawn_and_connect(config).await {
        Ok(mut bridge) => {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            // No correlation between the sim capture clock and PX4's
            // clock; PX4 also publishes no camera calibration.
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
            tracing::info!("PX4 camera sidecar up (FPV + chase)");
            (Some(rx), Some(bridge), forwarder)
        }
        Err(error) => {
            tracing::warn!(%error, "camera sidecar unavailable; no video");
            (None, None, None)
        }
    }
}

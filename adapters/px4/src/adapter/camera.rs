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

/// Builds the sidecar bridge configuration. FPV (source 0) and chase
/// (source 1) stay the world rig's fixed `/camera` and `/chase_camera` (the
/// bridge defaults); a vehicle CONFIGURED with a gimbal additionally
/// subscribes the gimbal payload's own feed (source 2): the CGO3 gimbal's
/// camera on the moving `camera_link` of the `x500_0` gimbal model, so it
/// pans and tilts with the quasimode independently of the forward FPV. A
/// gimbal-less vehicle subscribes NO third camera — the topic does not
/// exist in its world, and advertising a feed that never paints would be a
/// standing lie to the viewer. World name and model instance are fixed by
/// `sim/worlds/px4_flightdeck.sdf` (`default` / `x500_0`).
pub(crate) fn bridge_config(
    gimbal: bool,
    bin: std::path::PathBuf,
) -> pilotage_adapter_gazebo::BridgeConfig {
    let config = pilotage_adapter_gazebo::BridgeConfig::new("x500", bin);
    if gimbal {
        config.with_gimbal_camera_topic(
            "/world/default/model/x500_0/link/camera_link/sensor/camera/image",
        )
    } else {
        config
    }
}

/// Spawns the gz camera sidecar for the rig's `/camera` and
/// `/chase_camera` topics (plus the gimbal feed when the vehicle is
/// configured with one), degrading to no-video when it can't
/// (`PILOTAGE_PX4_CAMERA=off` disables the attempt).
#[allow(clippy::type_complexity)]
pub(crate) async fn spawn_camera_bridge(
    gimbal: bool,
) -> (
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
    let config = bridge_config(gimbal, bin);
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
            if gimbal {
                tracing::info!("PX4 camera sidecar up (FPV + chase + gimbal)");
            } else {
                tracing::info!("PX4 camera sidecar up (FPV + chase; no gimbal configured)");
            }
            (Some(rx), Some(bridge), forwarder)
        }
        Err(error) => {
            tracing::warn!(%error, "camera sidecar unavailable; no video");
            (None, None, None)
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::bridge_config;

    #[test]
    fn a_gimbal_vehicle_subscribes_the_gimbal_camera_topic() {
        let config = bridge_config(true, std::path::PathBuf::from("bridge-bin"));
        assert_eq!(
            config.gimbal_camera_topic.as_deref(),
            Some("/world/default/model/x500_0/link/camera_link/sensor/camera/image"),
        );
    }

    #[test]
    fn a_gimbal_less_vehicle_subscribes_no_gimbal_camera() {
        let config = bridge_config(false, std::path::PathBuf::from("bridge-bin"));
        assert_eq!(config.gimbal_camera_topic, None);
    }
}

//! The camera sidecar path for the PX4 adapter. Which producer feeds
//! the frames is a deployment property, selected by
//! `PILOTAGE_PX4_CAMERA`:
//!
//! - unset or `on`: Pilotage's C++ gz-transport sidecar delivers the
//!   flight-deck rig's `/camera` and `/chase_camera` frames (px4-gz).
//! - `xplane-plugin`: the in-simulator Pilotage camera plugin dials
//!   this process and delivers the vehicle camera view (FPV or gimbal
//!   payload) — the px4-xplane path, where the engine exposes no
//!   camera topic and the simulator owns the producer's lifetime.
//! - `off`: no video.
//!
//! Every producer speaks the same `pilotage.bridge.v1` protocol, so the
//! client, stamping, and media plumbing do not change with the producer.
//!
//! Frames are captured on the producer's own clock (gz simulation time,
//! or the capture host's monotonic clock) while PX4's flight state runs
//! on its own boot clock; no correlation between the two is available,
//! so every frame carries an unavailable clock mapping (ADR-0020) — a
//! consumer must gate conformal overlay off rather than draw against a
//! state it cannot align to the image.

use pilotage_adapter_api::FrameStamper;
use pilotage_adapter_api::{MeasurementClock, SourceIncarnation};

/// How this session sources camera frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CameraMode {
    /// The gz-transport sidecar (FPV + chase, plus gimbal when configured).
    Gazebo,
    /// The in-simulator camera plugin, which dials this process.
    XPlanePlugin,
    /// No video.
    Off,
}

/// Loopback port the X-Plane camera plugin dials.
const XPLANE_CAMERA_PORT: u16 = 45990;

/// Bounded frame-channel depth: small, so a slow media consumer drops
/// stale frames instead of growing latency.
const FRAME_CHANNEL_DEPTH: usize = 4;

/// Resolves `PILOTAGE_PX4_CAMERA` — an unknown value degrades to no
/// video with a warning rather than inventing a producer.
fn camera_mode() -> CameraMode {
    match std::env::var("PILOTAGE_PX4_CAMERA").as_deref() {
        Err(_) | Ok("on") => CameraMode::Gazebo,
        Ok("xplane-plugin") => CameraMode::XPlanePlugin,
        Ok("off") => CameraMode::Off,
        Ok(other) => {
            tracing::warn!(value = other, "unknown PILOTAGE_PX4_CAMERA; no video");
            CameraMode::Off
        }
    }
}

/// Builds the gz sidecar bridge configuration. FPV (source 0) and chase
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
) -> pilotage_sim_video::BridgeConfig {
    let config = pilotage_sim_video::BridgeConfig::new("x500", bin);
    if gimbal {
        config.with_gimbal_camera_topic(
            "/world/default/model/x500_0/link/camera_link/sensor/camera/image",
        )
    } else {
        config
    }
}

/// How a producer attaches, and the clock its capture stamps carry.
enum Producer {
    /// This process spawns the producer and accepts its dial-back.
    Spawned(pilotage_sim_video::BridgeConfig, MeasurementClock),
    /// The producer lives inside the simulator and dials this port;
    /// its lifetime belongs to the simulator, not to the session.
    Accepted(u16, MeasurementClock),
}

/// The producer attachment for `mode`; `None` means no video.
fn producer_for(
    mode: CameraMode,
    gimbal: bool,
    workspace_root: &std::path::Path,
) -> Option<Producer> {
    match mode {
        CameraMode::Gazebo => {
            let bin = workspace_root.join("adapters/gazebo/bridge/build/pilotage-gz-bridge");
            Some(Producer::Spawned(
                bridge_config(gimbal, bin),
                MeasurementClock::Simulation,
            ))
        }
        // The plugin stamps frames with the capture host's monotonic
        // clock: a simulator window has no clock a consumer can relate
        // to the flight state.
        CameraMode::XPlanePlugin => Some(Producer::Accepted(
            XPLANE_CAMERA_PORT,
            MeasurementClock::HostMonotonic,
        )),
        CameraMode::Off => None,
    }
}

/// Spawns the session's camera sidecar, degrading to no-video when it
/// can't (`PILOTAGE_PX4_CAMERA=off` disables the attempt).
#[allow(clippy::type_complexity)]
pub(crate) async fn spawn_camera_bridge(
    gimbal: bool,
) -> (
    Option<tokio::sync::mpsc::Receiver<pilotage_adapter_api::RawVideoFrame>>,
    Option<pilotage_sim_video::BridgeClient>,
    Option<tokio::task::JoinHandle<()>>,
) {
    let mode = camera_mode();
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    let Some(producer) = producer_for(mode, gimbal, &workspace_root) else {
        return (None, None, None);
    };
    let incarnation = SourceIncarnation::new(super::rand_incarnation());
    let (attached, clock) = match producer {
        Producer::Spawned(config, clock) => (
            pilotage_sim_video::BridgeClient::spawn_and_connect(config).await,
            clock,
        ),
        Producer::Accepted(port, clock) => {
            tracing::info!(port, "waiting for the in-simulator camera plugin");
            (
                pilotage_sim_video::BridgeClient::accept_producer(port, FRAME_CHANNEL_DEPTH).await,
                clock,
            )
        }
    };
    match attached {
        Ok(mut bridge) => {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            // No correlation between the capture clock and PX4's boot
            // clock; PX4 also publishes no camera calibration.
            let mut stamper = FrameStamper::new(
                incarnation,
                clock,
                pilotage_adapter_api::CaptureClockMapping::Unavailable,
                std::collections::BTreeMap::new(),
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
            tracing::info!(?mode, gimbal, "PX4 camera sidecar up");
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
    use super::{CameraMode, Producer, XPLANE_CAMERA_PORT, bridge_config, producer_for};
    use pilotage_adapter_api::MeasurementClock;

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

    #[test]
    fn producers_declare_their_attachment_and_clock() {
        let root = std::path::Path::new("/repo");
        // The gz sidecar is spawned by this process and stamps frames on
        // the simulation clock.
        match producer_for(CameraMode::Gazebo, false, root) {
            Some(Producer::Spawned(_, clock)) => {
                assert_eq!(clock, MeasurementClock::Simulation);
            }
            other => panic!("expected a spawned gz producer, got {:?}", other.is_some()),
        }
        // The in-simulator plugin dials in and stamps on the host clock:
        // a simulator window has no clock a consumer can relate to the
        // flight state.
        match producer_for(CameraMode::XPlanePlugin, false, root) {
            Some(Producer::Accepted(port, clock)) => {
                assert_eq!(port, XPLANE_CAMERA_PORT);
                assert_eq!(clock, MeasurementClock::HostMonotonic);
            }
            other => panic!("expected an accepted producer, got {:?}", other.is_some()),
        }
        assert!(producer_for(CameraMode::Off, false, root).is_none());
    }
}

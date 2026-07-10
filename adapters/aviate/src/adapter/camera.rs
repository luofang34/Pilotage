//! The gz camera sidecar path: Pilotage's own C++ gz-transport bridge
//! delivers the flight world's `/camera` and `/chase_camera` frames.

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
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    let bin = workspace_root.join("adapters/gazebo/bridge/build/pilotage-gz-bridge");
    let config = pilotage_adapter_gazebo::BridgeConfig::new("x500", bin);
    match pilotage_adapter_gazebo::BridgeClient::spawn_and_connect(config).await {
        Ok(mut bridge) => {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            let forwarder = bridge.take_frame_rx().map(|mut bridge_rx| {
                tokio::spawn(async move {
                    while let Some(frame) = bridge_rx.recv().await {
                        let raw = pilotage_adapter_gazebo::RawVideoFrame::from(frame);
                        if tx.send(raw).await.is_err() {
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

//! Constructs the real Gazebo adapter and wires its raw-frame receiver out to
//! the media task (ADR-0008): spawns the sidecar bridge, builds the session
//! engine from the adapter's advertised capabilities, and hands back the frame
//! stream the video downlink drains.

use std::path::PathBuf;
use std::time::Duration;

use pilotage_adapter_api::VehicleAdapter;
use pilotage_adapter_gazebo::{BridgeConfig, GazeboAdapter, RawVideoFrame};
use pilotage_protocol::VehicleId;
use pilotage_session::{SessionConfig, SessionEngine};
use pilotage_timing::StalenessPolicy;
use tokio::sync::mpsc;

use crate::error::HostError;

/// Gazebo model name of the diff-drive vehicle the bridge subscribes to.
const GAZEBO_VEHICLE_NAME: &str = "vehicle_blue";

/// Default path to the built C++ sidecar bridge binary, relative to the
/// workspace root, used when `PILOTAGE_GZ_BRIDGE_BIN` is unset. The CMake
/// build writes the binary here; an operator with a different layout points
/// the env var at it instead.
const DEFAULT_BRIDGE_REL: &str = "adapters/gazebo/bridge/build/pilotage-gz-bridge";

/// Builds the Gazebo adapter and session engine, returning the adapter's raw
/// frame receiver for the media task to drain.
///
/// # Errors
///
/// Returns [`HostError::GazeboAdapter`] if the sidecar bridge cannot be
/// spawned or its connection cannot be accepted.
pub async fn build_gazebo(
    vehicle: VehicleId,
    max_control_age: Duration,
) -> Result<(SessionEngine, GazeboAdapter, mpsc::Receiver<RawVideoFrame>), HostError> {
    let config = BridgeConfig::new(GAZEBO_VEHICLE_NAME, default_bridge_bin());
    let mut adapter = GazeboAdapter::new(vehicle, config)
        .await
        .map_err(HostError::GazeboAdapter)?;
    let frames = adapter
        .subscribe_frames()
        .ok_or(HostError::GazeboAdapter(gazebo_frames_already_taken()))?;
    let engine = build_engine(&adapter, max_control_age);
    Ok((engine, adapter, frames))
}

/// Resolves the default sidecar bridge binary path from the workspace root.
fn default_bridge_bin() -> PathBuf {
    workspace_root().join(DEFAULT_BRIDGE_REL)
}

/// Workspace root, two levels up from this crate's manifest
/// (`hosts/session-host`).
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// Constructs the video-frame error for the impossible "frames already taken"
/// case: `build_gazebo` is the sole caller of `subscribe_frames`, so this is a
/// defensive guard, not an expected path.
fn gazebo_frames_already_taken() -> pilotage_adapter_gazebo::GazeboAdapterError {
    pilotage_adapter_gazebo::GazeboAdapterError::ReaderTaskEnded {
        reason: "frame receiver already taken before the media task started".to_owned(),
    }
}

/// Builds the session engine from the adapter's advertised capabilities.
fn build_engine(adapter: &GazeboAdapter, max_control_age: Duration) -> SessionEngine {
    let capabilities = adapter.capabilities();
    let staleness = StalenessPolicy::new(max_control_age);
    let config = SessionConfig::new(pilotage_protocol::SCHEMA_VERSION, env!("CARGO_PKG_VERSION"));
    SessionEngine::new(capabilities, staleness, config)
}

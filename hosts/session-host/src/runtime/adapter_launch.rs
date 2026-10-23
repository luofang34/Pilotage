//! Per-adapter construction and runtime spawning: builds the selected
//! vehicle adapter, chooses the session's video source (the adapter's own
//! frames or the simulator camera attachment), and spawns the shared
//! `run_until_shutdown` lifecycle.

use pilotage_adapter_api::{RawVideoFrame, VehicleAdapter};
use pilotage_session::SessionEngine;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use wtransport::Endpoint;

use crate::cli::AdapterKind;
use crate::error::HostError;

use super::engine_actor::ToEngine;
#[cfg(feature = "sim")]
use super::sim_camera;
use super::{
    HOST_VEHICLE, RuntimeOptions, ardupilot_config, aviate_profile, betaflight_config,
    build_engine, media, px4_config, run_until_shutdown, run_with_media_until_shutdown,
};
#[cfg(feature = "sim")]
use super::{MAX_CONTROL_AGE, build_reference, gazebo_launch};

/// The host side of a session runtime: what every adapter path hands to
/// [`spawn_runtime`] besides the adapter itself.
struct HostSide {
    join_token: Option<pilotage_protocol::JoinToken>,
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
    start: tokio::time::Instant,
}

/// The session's video: a frame source for the media task, and the task
/// that feeds it when that is not the adapter.
struct SessionVideo {
    frames: Option<mpsc::Receiver<RawVideoFrame>>,
    producer: Option<(&'static str, JoinHandle<()>)>,
}

/// Builds the chosen adapter and spawns the per-adapter
/// `run_until_shutdown` task.
pub(super) async fn spawn_adapter_runtime(
    adapter: AdapterKind,
    mut options: RuntimeOptions,
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
    start: tokio::time::Instant,
) -> Result<JoinHandle<()>, HostError> {
    let sim_camera_port = options.sim_camera_port.take();
    let host = HostSide {
        join_token: options.network.join_token.clone(),
        endpoint,
        engine_tx,
        engine_rx,
        shutdown_rx,
        start,
    };
    launch_adapter(adapter, options, sim_camera_port, host).await
}

async fn launch_adapter(
    adapter: AdapterKind,
    options: RuntimeOptions,
    sim_camera_port: Option<u16>,
    host: HostSide,
) -> Result<JoinHandle<()>, HostError> {
    match adapter {
        #[cfg(feature = "sim")]
        AdapterKind::Reference => {
            let (engine, adapter) = build_reference(options);
            let video = session_video("reference", None, sim_camera_port, &host.engine_tx).await?;
            Ok(spawn_runtime(engine, adapter, video, host))
        }
        #[cfg(feature = "sim")]
        AdapterKind::Gazebo => {
            let (engine, adapter, frames) =
                gazebo_launch::build_gazebo(HOST_VEHICLE, MAX_CONTROL_AGE).await?;
            let video =
                session_video("gazebo", Some(frames), sim_camera_port, &host.engine_tx).await?;
            Ok(spawn_runtime(engine, adapter, video, host))
        }
        #[cfg(feature = "sim")]
        AdapterKind::SimMulticopter => {
            let config = super::sim_multicopter::config_from_env()?;
            tracing::info!(
                name = %config.display_name(),
                lat_deg = config.launch.lat_deg,
                lon_deg = config.launch.lon_deg,
                alt_m = config.launch.alt_m,
                "simulated multicopter on the DJI control path"
            );
            let mut adapter =
                pilotage_adapter_sim_multicopter::SimMulticopterAdapter::new(HOST_VEHICLE, config);
            let engine = build_engine(&adapter, options);
            let frames = adapter.subscribe_frames();
            let video =
                session_video("sim-multicopter", frames, sim_camera_port, &host.engine_tx).await?;
            Ok(spawn_runtime(engine, adapter, video, host))
        }
        #[cfg(not(feature = "sim"))]
        AdapterKind::SimMulticopter => Err(HostError::AdapterNotInBuild {
            adapter: "sim-multicopter",
        }),
        #[cfg(not(feature = "sim"))]
        AdapterKind::Reference => Err(HostError::AdapterNotInBuild {
            adapter: "reference",
        }),
        #[cfg(not(feature = "sim"))]
        AdapterKind::Gazebo => Err(HostError::AdapterNotInBuild { adapter: "gazebo" }),
        AdapterKind::Aviate => {
            let mut adapter = start_aviate().await?;
            let engine = build_engine(&adapter, options);
            let frames = adapter.subscribe_frames();
            let video = session_video("aviate", frames, sim_camera_port, &host.engine_tx).await?;
            Ok(spawn_runtime(engine, adapter, video, host))
        }
        AdapterKind::Px4 => {
            let config = px4_config::from_env()?;
            let mut adapter = pilotage_adapter_px4::Px4Adapter::start(HOST_VEHICLE, config)
                .await
                .map_err(HostError::Px4Adapter)?;
            let engine = build_engine(&adapter, options);
            let frames = adapter.subscribe_frames();
            let video = session_video("px4", frames, sim_camera_port, &host.engine_tx).await?;
            Ok(spawn_runtime(engine, adapter, video, host))
        }
        AdapterKind::ArduPilot => {
            let config = ardupilot_config::from_env()?;
            let adapter = pilotage_adapter_ardupilot::ArduPilotAdapter::start(HOST_VEHICLE, config)
                .await
                .map_err(HostError::ArduPilotAdapter)?;
            let engine = build_engine(&adapter, options);
            let video = session_video("ardupilot", None, sim_camera_port, &host.engine_tx).await?;
            Ok(spawn_runtime(engine, adapter, video, host))
        }
        AdapterKind::Betaflight => {
            // The adapter's link task connects and reconnects on its own, so
            // the host comes up before (or without) the flight controller.
            let config = betaflight_config::from_env()?;
            let adapter =
                pilotage_adapter_betaflight::BetaflightAdapter::start(HOST_VEHICLE, config);
            let engine = build_engine(&adapter, options);
            let video = session_video("betaflight", None, sim_camera_port, &host.engine_tx).await?;
            Ok(spawn_runtime(engine, adapter, video, host))
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        AdapterKind::Dji => spawn_dji_runtime_blocking(options, sim_camera_port, host),
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        AdapterKind::Dji => Err(HostError::AdapterNotInBuild { adapter: "dji" }),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn spawn_dji_runtime_blocking(
    options: RuntimeOptions,
    sim_camera_port: Option<u16>,
    host: HostSide,
) -> Result<JoinHandle<()>, HostError> {
    if sim_camera_port.is_some() {
        return Err(HostError::SimCameraConflict { adapter: "dji" });
    }
    let super::dji::DjiRuntime {
        adapter,
        frames,
        producer,
    } = super::dji::start_blocking(HOST_VEHICLE, host.start.into_std())?;
    let engine = build_engine(&adapter, options);
    let video = SessionVideo {
        frames,
        producer: producer.map(|task| ("dji-video-decoder", task)),
    };
    Ok(spawn_runtime(engine, adapter, video, host))
}

/// Chooses the session's video. With a simulator camera port configured
/// the attachment supplies it, and an adapter that already produces video
/// is refused: two producers would claim the same camera sources.
async fn session_video(
    adapter: &'static str,
    adapter_frames: Option<mpsc::Receiver<RawVideoFrame>>,
    sim_camera_port: Option<u16>,
    engine_tx: &mpsc::Sender<ToEngine>,
) -> Result<SessionVideo, HostError> {
    let Some(port) = sim_camera_port else {
        return Ok(SessionVideo {
            frames: adapter_frames,
            producer: None,
        });
    };
    if adapter_frames.is_some() {
        return Err(HostError::SimCameraConflict { adapter });
    }
    attach_sim_camera(port, engine_tx).await
}

#[cfg(feature = "sim")]
async fn attach_sim_camera(
    port: u16,
    engine_tx: &mpsc::Sender<ToEngine>,
) -> Result<SessionVideo, HostError> {
    let listener = sim_camera::bind(port).await?;
    tracing::info!(port, "sim camera attachment waiting for its producer");
    let (frames, producer) = sim_camera::spawn(listener, engine_tx);
    Ok(SessionVideo {
        frames: Some(frames),
        producer: Some(("sim-camera", producer)),
    })
}

#[cfg(not(feature = "sim"))]
async fn attach_sim_camera(
    _port: u16,
    _engine_tx: &mpsc::Sender<ToEngine>,
) -> Result<SessionVideo, HostError> {
    Err(HostError::SimCameraNotInBuild)
}

/// Spawns an adapter's runtime, wiring the media task only when the
/// session has a video source.
fn spawn_runtime<A>(
    engine: SessionEngine,
    adapter: A,
    video: SessionVideo,
    host: HostSide,
) -> JoinHandle<()>
where
    A: VehicleAdapter + Send + 'static,
{
    let HostSide {
        join_token,
        endpoint,
        engine_tx,
        engine_rx,
        shutdown_rx,
        start,
    } = host;
    let Some(frames) = video.frames else {
        return tokio::spawn(run_until_shutdown(
            endpoint,
            engine,
            adapter,
            None,
            engine_tx,
            engine_rx,
            shutdown_rx,
            start,
            join_token,
        ));
    };
    let (media, media_task) = media::spawn_media_task(frames, start);
    // Own the tasks before spawning the future so cancellation before its
    // first poll also stops the decoder and media tasks.
    let media_tasks = super::video_lifecycle::VideoTasks::new(media_task, video.producer);
    tokio::spawn(run_with_media_until_shutdown(
        endpoint,
        engine,
        adapter,
        media,
        media_tasks,
        engine_tx,
        engine_rx,
        shutdown_rx,
        start,
        join_token,
    ))
}

/// Builds the Aviate adapter from its environment.
async fn start_aviate() -> Result<pilotage_adapter_aviate::AviateAdapter, HostError> {
    // PILOTAGE_AVIATE_PROFILE selects the session profile (LINK-04):
    // "physical" (FC estimate + FC state; no truth), the default
    // "simulation" (estimate + FC state, plus the truth oracle when the
    // co-located shm block attaches), or "oracle-only" (truth stream
    // only; no uplink, no operational control). Parsing fails closed and
    // Physical gets the conservative link configuration.
    let profile = aviate_profile::profile_from_env(std::env::var("PILOTAGE_AVIATE_PROFILE"))?;
    let control_feel = aviate_profile::control_feel_from_env_blocking(
        profile,
        std::env::var_os("PILOTAGE_AVIATE_CONTROL_FEEL_PROFILE"),
    )?;
    pilotage_adapter_aviate::AviateAdapter::start_with_control_feel(
        HOST_VEHICLE,
        profile,
        aviate_profile::truth_from_env(std::env::var("PILOTAGE_AVIATE_TRUTH"))?,
        aviate_profile::link_config_from_env(profile, std::env::var("PILOTAGE_AVIATE_ADDR"))?,
        control_feel,
    )
    .await
    .map_err(HostError::AviateAdapter)
}

#[cfg(test)]
mod tests;

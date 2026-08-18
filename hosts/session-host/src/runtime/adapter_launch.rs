//! Per-adapter construction and runtime spawning: builds the selected
//! vehicle adapter (and its media task when it exposes video frames) and
//! spawns the shared `run_until_shutdown` lifecycle.

use tokio::sync::{mpsc, oneshot};
use wtransport::Endpoint;

use crate::cli::AdapterKind;
use crate::error::HostError;

use super::engine_actor::ToEngine;
use super::{
    HOST_VEHICLE, RuntimeOptions, aviate_profile, build_engine, media, px4_config,
    run_until_shutdown, run_with_media_until_shutdown,
};
#[cfg(feature = "sim")]
use super::{MAX_CONTROL_AGE, build_reference, gazebo_launch};

/// Builds the chosen adapter (and, for Gazebo, its media task) and spawns the
/// per-adapter `run_until_shutdown` task.
pub(super) async fn spawn_adapter_runtime(
    adapter: AdapterKind,
    options: RuntimeOptions,
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
    start: tokio::time::Instant,
) -> Result<tokio::task::JoinHandle<()>, HostError> {
    match adapter {
        #[cfg(feature = "sim")]
        AdapterKind::Reference => {
            let (engine, adapter) = build_reference(options);
            Ok(tokio::spawn(run_until_shutdown(
                endpoint,
                engine,
                adapter,
                None,
                engine_tx,
                engine_rx,
                shutdown_rx,
                start,
            )))
        }
        #[cfg(feature = "sim")]
        AdapterKind::Gazebo => {
            let (engine, adapter, frames) =
                gazebo_launch::build_gazebo(HOST_VEHICLE, MAX_CONTROL_AGE).await?;
            let (media, media_task) = media::spawn_media_task(frames, start);
            Ok(tokio::spawn(run_with_media_until_shutdown(
                endpoint,
                engine,
                adapter,
                media,
                media_task,
                engine_tx,
                engine_rx,
                shutdown_rx,
                start,
            )))
        }
        #[cfg(not(feature = "sim"))]
        AdapterKind::Reference => Err(HostError::AdapterNotInBuild {
            adapter: "reference",
        }),
        #[cfg(not(feature = "sim"))]
        AdapterKind::Gazebo => Err(HostError::AdapterNotInBuild { adapter: "gazebo" }),
        AdapterKind::Aviate => {
            spawn_aviate_runtime(endpoint, options, engine_tx, engine_rx, shutdown_rx, start).await
        }
        AdapterKind::Px4 => {
            spawn_px4_runtime(endpoint, options, engine_tx, engine_rx, shutdown_rx, start).await
        }
    }
}

/// Builds the PX4 adapter and spawns its runtime, wiring the media task only
/// when the adapter exposes a video frame source.
async fn spawn_px4_runtime(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    options: RuntimeOptions,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
    start: tokio::time::Instant,
) -> Result<tokio::task::JoinHandle<()>, HostError> {
    let config = px4_config::from_env()?;
    let mut adapter = pilotage_adapter_px4::Px4Adapter::start(HOST_VEHICLE, config)
        .await
        .map_err(HostError::Px4Adapter)?;
    let engine = build_engine(&adapter, options);
    match adapter.subscribe_frames() {
        Some(frames) => {
            let (media, media_task) = media::spawn_media_task(frames, start);
            Ok(tokio::spawn(run_with_media_until_shutdown(
                endpoint,
                engine,
                adapter,
                media,
                media_task,
                engine_tx,
                engine_rx,
                shutdown_rx,
                start,
            )))
        }
        None => Ok(tokio::spawn(run_until_shutdown(
            endpoint,
            engine,
            adapter,
            None,
            engine_tx,
            engine_rx,
            shutdown_rx,
            start,
        ))),
    }
}

/// Builds the Aviate adapter and spawns its runtime, wiring the media task only
/// when the adapter exposes a video frame source.
async fn spawn_aviate_runtime(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    options: RuntimeOptions,
    engine_tx: mpsc::Sender<ToEngine>,
    engine_rx: mpsc::Receiver<ToEngine>,
    shutdown_rx: oneshot::Receiver<()>,
    start: tokio::time::Instant,
) -> Result<tokio::task::JoinHandle<()>, HostError> {
    // PILOTAGE_AVIATE_PROFILE selects the session profile (LINK-04):
    // "physical" (FC estimate + FC state; no truth), the default
    // "simulation" (estimate + FC state, plus the truth oracle when the
    // co-located shm block attaches), or "oracle-only" (truth stream
    // only; no uplink, no operational control). Parsing fails closed and
    // Physical gets the conservative link configuration.
    let profile = aviate_profile::profile_from_env(std::env::var("PILOTAGE_AVIATE_PROFILE"))?;
    let mut adapter = pilotage_adapter_aviate::AviateAdapter::start(
        HOST_VEHICLE,
        profile,
        aviate_profile::link_config(profile),
    )
    .await
    .map_err(HostError::AviateAdapter)?;
    let engine = build_engine(&adapter, options);
    match adapter.subscribe_frames() {
        Some(frames) => {
            let (media, media_task) = media::spawn_media_task(frames, start);
            Ok(tokio::spawn(run_with_media_until_shutdown(
                endpoint,
                engine,
                adapter,
                media,
                media_task,
                engine_tx,
                engine_rx,
                shutdown_rx,
                start,
            )))
        }
        None => Ok(tokio::spawn(run_until_shutdown(
            endpoint,
            engine,
            adapter,
            None,
            engine_tx,
            engine_rx,
            shutdown_rx,
            start,
        ))),
    }
}

//! Owns the localhost TCP link to the C++ gz-transport sidecar bridge
//! (ADR-0008): binds a listener, spawns the bridge child, accepts its
//! connection, then runs a background reader task (odometry -> shared
//! latest-value state; camera frames -> a bounded channel) and a writer task
//! (outbound `BridgeControl`).
//!
//! This module is intentionally I/O-heavy (`adapters/` is exempt from the
//! sans-IO rule, ADR-0002): it is the only place in this crate that touches a
//! socket or a child process. No raw gz-transport type crosses into
//! `pilotage-protocol`; only the internal `pilotage.bridge.v1` wire types do.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use prost::Message;
use tokio::io::{AsyncWriteExt, WriteHalf};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::error::GazeboAdapterError;
use crate::framing::read_envelope;
use crate::wire::{BridgeControl, BridgeEnvelope, BridgeFrame, BridgeOdometry, bridge_envelope};

/// Environment variable overriding the sidecar bridge binary path.
pub const BRIDGE_BIN_ENV: &str = "PILOTAGE_GZ_BRIDGE_BIN";

/// Default bounded depth of the raw-frame channel. Small so a slow media
/// consumer drops stale frames rather than growing memory or adding latency.
const DEFAULT_FRAME_CHANNEL_DEPTH: usize = 4;

/// Configuration for spawning and connecting to the sidecar bridge.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Path to the built C++ bridge binary. Overridden by [`BRIDGE_BIN_ENV`]
    /// when that environment variable is set.
    pub bridge_bin: PathBuf,
    /// Gazebo model name passed to the bridge as `--vehicle`.
    pub vehicle_name: String,
    /// FPV (`camera_id = 0`) gz image topic. `None` leaves the bridge on its
    /// own default (`/camera`); worlds where the onboard view is a moving
    /// gimbal camera on a scoped topic set it explicitly.
    pub camera_topic: Option<String>,
    /// Gimbal payload (`camera_id = 2`) gz image topic. `None` when the vehicle
    /// carries no gimbal, so the bridge subscribes no third camera.
    pub gimbal_camera_topic: Option<String>,
    /// Bounded depth of the raw-frame channel.
    pub frame_channel_depth: usize,
}

impl BridgeConfig {
    /// Builds a config for `vehicle_name`, resolving the bridge binary path
    /// from [`BRIDGE_BIN_ENV`] if set, else from `default_bridge_bin`.
    #[must_use]
    pub fn new(vehicle_name: impl Into<String>, default_bridge_bin: PathBuf) -> Self {
        let bridge_bin = std::env::var_os(BRIDGE_BIN_ENV).map_or(default_bridge_bin, PathBuf::from);
        Self {
            bridge_bin,
            vehicle_name: vehicle_name.into(),
            camera_topic: None,
            gimbal_camera_topic: None,
            frame_channel_depth: DEFAULT_FRAME_CHANNEL_DEPTH,
        }
    }

    /// Overrides the FPV camera gz topic (the bridge's `--camera-topic`), so
    /// the onboard view can be sourced from a moving gimbal camera rather than
    /// the bridge's fixed `/camera` default.
    #[must_use]
    pub fn with_camera_topic(mut self, topic: impl Into<String>) -> Self {
        self.camera_topic = Some(topic.into());
        self
    }

    /// Adds the gimbal payload camera (`camera_id = 2`, the bridge's
    /// `--gimbal-camera-topic`) as a third video source, distinct from the FPV
    /// and chase views, so the gimbal's own pannable view has its own feed.
    #[must_use]
    pub fn with_gimbal_camera_topic(mut self, topic: impl Into<String>) -> Self {
        self.gimbal_camera_topic = Some(topic.into());
        self
    }
}

/// Latest cached odometry read from the sidecar bridge, shared between the
/// background reader task and `sample_telemetry` callers.
///
/// Camera frames are delivered separately through a bounded channel (see
/// [`BridgeClient::take_frame_rx`]) rather than this struct: raw video does
/// not fit the pull-based, latest-value `sample_telemetry` shape (ADR-0008).
#[derive(Debug, Clone, Default)]
pub struct LatestBridgeState {
    /// Most recent odometry sample, if any has arrived yet.
    pub odometry: Option<BridgeOdometry>,
}

/// Liveness of the background reader task that feeds odometry and frames.
///
/// Once the reader exits (bridge EOF, a read/decode error, or the sidecar
/// child dying), the odometry cache can never advance again. Callers poll
/// [`BridgeClient::reader_health`] to distinguish a live-but-idle cache from a
/// permanently frozen one, so stale telemetry is not mistaken for current.
#[derive(Debug, Clone)]
enum ReaderHealth {
    /// The reader loop is still running.
    Alive,
    /// The reader loop has exited; the string describes why.
    Ended(String),
}

/// A live connection to the gz-transport sidecar bridge and the child process
/// hosting it.
///
/// Dropping the client kills the bridge child and aborts both background
/// tasks, so no orphan process or task outlives the adapter.
#[derive(Debug)]
pub struct BridgeClient {
    child: Option<Child>,
    reader_task: JoinHandle<()>,
    writer_task: JoinHandle<()>,
    control_tx: watch::Sender<Option<BridgeControl>>,
    state_rx: watch::Receiver<LatestBridgeState>,
    reader_health_rx: watch::Receiver<ReaderHealth>,
    frame_rx: Option<mpsc::Receiver<BridgeFrame>>,
    dropped_frames: Arc<AtomicU64>,
}

impl BridgeClient {
    /// Binds a localhost listener, spawns the sidecar bridge child pointed at
    /// the bound port, accepts its inbound connection, and starts the reader
    /// and writer tasks.
    ///
    /// # Errors
    ///
    /// Returns a [`GazeboAdapterError`] if the listener cannot bind, the child
    /// cannot be spawned, or the inbound connection cannot be accepted.
    pub async fn spawn_and_connect(config: BridgeConfig) -> Result<Self, GazeboAdapterError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|source| GazeboAdapterError::ListenerBind { source })?;
        let local_addr = listener
            .local_addr()
            .map_err(|source| GazeboAdapterError::ListenerAddr { source })?;

        let child = Self::spawn_child(&config, local_addr.port())?;
        let stream = Self::accept(&listener, local_addr).await?;
        Ok(Self::from_stream(
            stream,
            Some(child),
            config.frame_channel_depth,
        ))
    }

    /// Wires the reader/writer tasks around an already-connected stream. Shared
    /// by [`Self::spawn_and_connect`] and, in tests, an in-process fake bridge.
    fn from_stream(
        stream: tokio::net::TcpStream,
        child: Option<Child>,
        frame_channel_depth: usize,
    ) -> Self {
        let (read_half, write_half) = tokio::io::split(stream);
        let (state_tx, state_rx) = watch::channel(LatestBridgeState::default());
        let (frame_tx, frame_rx) = mpsc::channel(frame_channel_depth.max(1));
        let (control_tx, control_rx) = watch::channel::<Option<BridgeControl>>(None);
        let (reader_health_tx, reader_health_rx) = watch::channel(ReaderHealth::Alive);
        let dropped_frames = Arc::new(AtomicU64::new(0));

        let reader_task = tokio::spawn(reader_loop(
            read_half,
            state_tx,
            frame_tx,
            reader_health_tx,
            Arc::clone(&dropped_frames),
        ));
        let writer_task = tokio::spawn(writer_loop(write_half, control_rx));

        Self {
            child,
            reader_task,
            writer_task,
            control_tx,
            state_rx,
            reader_health_rx,
            frame_rx: Some(frame_rx),
            dropped_frames,
        }
    }

    /// Builds the sidecar bridge command line for `config` on `port`. Split out
    /// from [`Self::spawn_child`] so the argument list — notably the optional
    /// `--camera-topic` override that points FPV at a moving gimbal camera — is
    /// unit-testable without spawning a process.
    fn bridge_command(config: &BridgeConfig, port: u16) -> Command {
        let mut command = Command::new(&config.bridge_bin);
        command
            .arg("--port")
            .arg(port.to_string())
            .arg("--vehicle")
            .arg(&config.vehicle_name);
        if let Some(camera_topic) = &config.camera_topic {
            command.arg("--camera-topic").arg(camera_topic);
        }
        if let Some(gimbal_camera_topic) = &config.gimbal_camera_topic {
            command
                .arg("--gimbal-camera-topic")
                .arg(gimbal_camera_topic);
        }
        command
    }

    fn spawn_child(config: &BridgeConfig, port: u16) -> Result<Child, GazeboAdapterError> {
        Self::bridge_command(config, port)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| GazeboAdapterError::BridgeSpawn {
                path: config.bridge_bin.display().to_string(),
                source,
            })
    }

    async fn accept(
        listener: &TcpListener,
        local_addr: SocketAddr,
    ) -> Result<tokio::net::TcpStream, GazeboAdapterError> {
        let (stream, _peer) =
            listener
                .accept()
                .await
                .map_err(|source| GazeboAdapterError::BridgeAccept {
                    addr: local_addr.to_string(),
                    source,
                })?;
        Ok(stream)
    }

    /// Returns the latest cached bridge state (odometry).
    #[must_use]
    pub fn latest_cached(&self) -> LatestBridgeState {
        self.state_rx.borrow().clone()
    }

    /// Reports whether the background reader task is still consuming the bridge
    /// connection.
    ///
    /// The reader is the sole updater of the odometry cache and frame channel;
    /// once it exits, [`Self::latest_cached`] can never advance again. Callers
    /// poll this to detect that liveness gap rather than silently trusting a
    /// frozen telemetry cache (a teleop safety concern).
    ///
    /// # Errors
    ///
    /// Returns [`GazeboAdapterError::ReaderTaskEnded`] once the reader loop has
    /// exited (bridge EOF, a read/decode error, or the sidecar child dying),
    /// carrying the reason it ended.
    pub fn reader_health(&self) -> Result<(), GazeboAdapterError> {
        match &*self.reader_health_rx.borrow() {
            ReaderHealth::Alive => Ok(()),
            ReaderHealth::Ended(reason) => Err(GazeboAdapterError::ReaderTaskEnded {
                reason: reason.clone(),
            }),
        }
    }

    /// Publishes an outbound `BridgeControl` as the single latest command for
    /// the writer task without blocking on the socket. Returns `false` if the
    /// writer path is gone (the bridge disconnected), so callers can surface a
    /// rejection.
    #[must_use]
    pub fn try_send_control(&self, control: BridgeControl) -> bool {
        // Latest-valid-value: a single-slot watch always overwrites, so a
        // wedged writer never drains stale commands and the newest control is
        // never the one dropped (ADR-0009). `send` fails only once every
        // receiver is gone (the writer task exited), which is the sole "link
        // closed" signal callers care about.
        self.control_tx.send(Some(control)).is_ok()
    }

    /// Takes the raw-frame receiver, if not already taken. Frames are exposed
    /// here rather than through `sample_telemetry` because streaming video is
    /// backpressure-sensitive and does not fit the pull-based trait model.
    pub fn take_frame_rx(&mut self) -> Option<mpsc::Receiver<BridgeFrame>> {
        self.frame_rx.take()
    }

    /// Number of camera frames dropped because the frame channel was full.
    #[must_use]
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }
}

impl Drop for BridgeClient {
    fn drop(&mut self) {
        // Abort the tasks first so neither races the socket teardown, then let
        // `kill_on_drop` reap the child. No orphan process or task survives.
        self.reader_task.abort();
        self.writer_task.abort();
        if let Some(child) = self.child.as_mut()
            && let Err(err) = child.start_kill()
        {
            warn!(error = %err, "failed to signal sidecar bridge child on drop");
        }
    }
}

/// Reads length-delimited envelopes until EOF or error: odometry updates the
/// shared latest-value state; frames go to the bounded channel, counting drops.
///
/// On every exit path it publishes an [`ReaderHealth::Ended`] status carrying
/// the reason, so `reader_health` can surface the liveness loss instead of
/// letting `sample_telemetry` return a frozen odometry cache forever.
async fn reader_loop(
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
            state_tx.send_replace(LatestBridgeState {
                odometry: Some(odometry),
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

#[cfg(test)]
impl BridgeClient {
    /// Wires a client around a caller-supplied stream with no child process,
    /// for in-process fake-bridge tests.
    pub(crate) fn connect_stream_for_test(stream: tokio::net::TcpStream) -> Self {
        Self::from_stream(stream, None, DEFAULT_FRAME_CHANNEL_DEPTH)
    }
}

/// Writes the latest published control as a length-delimited envelope whenever
/// it changes. A slow socket coalesces intervening updates to the newest value
/// (latest-valid-value, ADR-0009). Exits on channel close (client dropped) or a
/// socket write error.
async fn writer_loop(
    mut write_half: WriteHalf<tokio::net::TcpStream>,
    mut control_rx: watch::Receiver<Option<BridgeControl>>,
) {
    while control_rx.changed().await.is_ok() {
        let Some(control) = *control_rx.borrow_and_update() else {
            continue;
        };
        let envelope = BridgeEnvelope {
            payload: Some(bridge_envelope::Payload::Control(control)),
        };
        let bytes = envelope.encode_length_delimited_to_vec();
        if let Err(err) = write_half.write_all(&bytes).await {
            warn!(error = %err, "sidecar bridge write failed; stopping writer");
            return;
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::PathBuf;

    use super::{BridgeClient, BridgeConfig};

    fn bridge_args(config: &BridgeConfig) -> Vec<String> {
        BridgeClient::bridge_command(config, 1234)
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn camera_topic_override_points_fpv_at_the_gimbal_camera() {
        let base = BridgeConfig::new("x500", PathBuf::from("/nonexistent/pilotage-gz-bridge"));
        // By default the bridge keeps its own `/camera` topic (no override).
        assert!(
            !bridge_args(&base).iter().any(|arg| arg == "--camera-topic"),
            "no --camera-topic by default"
        );

        // With an override, the exact gimbal-camera topic is passed through.
        let topic = "/world/default/model/x500_0/link/camera_link/sensor/camera/image";
        let args = bridge_args(&base.with_camera_topic(topic));
        let idx = args
            .iter()
            .position(|arg| arg == "--camera-topic")
            .expect("--camera-topic present");
        assert_eq!(args.get(idx + 1).map(String::as_str), Some(topic));
    }

    #[test]
    fn a_gimbal_camera_topic_adds_the_third_camera_flag() {
        let base = BridgeConfig::new("x500", PathBuf::from("/nonexistent/pilotage-gz-bridge"));
        // No gimbal camera by default: a bare airframe subscribes no third feed.
        assert!(
            !bridge_args(&base)
                .iter()
                .any(|arg| arg == "--gimbal-camera-topic"),
            "no --gimbal-camera-topic by default"
        );

        let topic = "/world/default/model/x500_0/link/camera_link/sensor/camera/image";
        let args = bridge_args(&base.with_gimbal_camera_topic(topic));
        let idx = args
            .iter()
            .position(|arg| arg == "--gimbal-camera-topic")
            .expect("--gimbal-camera-topic present");
        assert_eq!(args.get(idx + 1).map(String::as_str), Some(topic));
    }
}

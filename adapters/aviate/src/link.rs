//! The UDP MAVLink link task: receives Aviate telemetry, caches the
//! latest estimate, and (in router mode) keeps this endpoint registered
//! with GCS heartbeats.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::error::AviateAdapterError;
use crate::mavlink::{AviateMessage, encode_gcs_heartbeat, parse_datagram};

/// Where the Aviate MAVLink telemetry is reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    /// The MAVLink GCS endpoint. If this port is free the adapter binds
    /// it directly (the FC pushes datagrams straight at it); if it is
    /// taken (a MAVLink router owns it), the adapter binds an ephemeral
    /// port and registers with the router via 1 Hz GCS heartbeats.
    pub endpoint: SocketAddr,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            endpoint: SocketAddr::from(([127, 0, 0, 1], 14550)),
        }
    }
}

/// Latest state received from the FC, with receive stamps so staleness
/// can propagate to consumers (ADR-0018: loss of data marks groups
/// stale rather than freezing them).
#[derive(Debug, Default)]
pub struct LatestAviate {
    /// The vehicle system id this link is locked onto. A routed link
    /// carries several vehicles plus other GCS peers; the first system
    /// id to deliver an estimate wins and everything else is ignored
    /// (one adapter, one vehicle — ADR-0008).
    pub locked_sysid: Option<u8>,
    /// Latest attitude estimate: quaternion (w,x,y,z), body rates,
    /// FC boot time, receive stamp.
    pub attitude: Option<AttitudeUpdate>,
    /// Latest NED kinematics: position, velocity, FC boot time, receive
    /// stamp.
    pub kinematics: Option<KinematicsUpdate>,
    /// Receive stamp of the last FC heartbeat.
    pub last_heartbeat: Option<Instant>,
    /// Total decoded frames.
    pub decoded: u64,
    /// Total CRC failures (a correctness signal, logged).
    pub crc_failures: u64,
    /// Total structurally-valid frames with unknown ids.
    pub unknown_ids: u64,
}

/// One attitude update with its receive stamp.
#[derive(Debug, Clone, Copy)]
pub struct AttitudeUpdate {
    /// Quaternion (w, x, y, z), body FRD → world NED.
    pub quat_wxyz: [f32; 4],
    /// Body rates (p, q, r) rad/s.
    pub rates_rps: [f32; 3],
    /// When this update was received.
    pub received_at: Instant,
}

/// One kinematics update with its receive stamp.
#[derive(Debug, Clone, Copy)]
pub struct KinematicsUpdate {
    /// Position NED, meters.
    pub pos_ned_m: [f32; 3],
    /// Velocity NED, m/s.
    pub vel_ned_mps: [f32; 3],
    /// Milliseconds since FC boot.
    pub time_boot_ms: u32,
    /// When this update was received.
    pub received_at: Instant,
}

/// A running MAVLink link: the receive task plus the shared latest-state
/// cache the adapter samples from.
#[derive(Debug)]
pub struct AviateLink {
    state: Arc<Mutex<LatestAviate>>,
    task: JoinHandle<()>,
}

impl AviateLink {
    /// Binds the socket (direct or router mode) and spawns the receive
    /// task.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError::Bind`] when no socket can be bound.
    pub async fn start(config: LinkConfig) -> Result<Self, AviateAdapterError> {
        let (socket, router_mode) = match UdpSocket::bind(config.endpoint).await {
            Ok(socket) => (socket, false),
            Err(direct_err) => {
                debug!(%direct_err, "MAVLink endpoint taken; assuming a router owns it");
                let socket = UdpSocket::bind((config.endpoint.ip(), 0))
                    .await
                    .map_err(|source| AviateAdapterError::Bind { source })?;
                (socket, true)
            }
        };
        info!(
            mode = if router_mode { "router" } else { "direct" },
            endpoint = %config.endpoint,
            "Aviate MAVLink link listening"
        );
        let state = Arc::new(Mutex::new(LatestAviate::default()));
        let task = tokio::spawn(run_link(
            socket,
            config.endpoint,
            router_mode,
            state.clone(),
        ));
        Ok(Self { state, task })
    }

    /// The shared latest-state cache.
    pub fn state(&self) -> Arc<Mutex<LatestAviate>> {
        self.state.clone()
    }
}

impl Drop for AviateLink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_link(
    socket: UdpSocket,
    endpoint: SocketAddr,
    router_mode: bool,
    state: Arc<Mutex<LatestAviate>>,
) {
    let mut buf = vec![0u8; 2048];
    let mut messages = Vec::with_capacity(8);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    let mut seq: u8 = 0;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if router_mode {
                    let frame = encode_gcs_heartbeat(seq);
                    seq = seq.wrapping_add(1);
                    if let Err(error) = socket.send_to(&frame, endpoint).await {
                        warn!(%error, "GCS heartbeat send failed");
                    }
                }
            }
            received = socket.recv_from(&mut buf) => {
                match received {
                    Ok((len, _from)) => {
                        messages.clear();
                        let stats = parse_datagram(buf.get(..len).unwrap_or(&[]), &mut messages);
                        apply_messages(&state, &messages, stats.crc_failures, stats.unknown_ids);
                        if stats.crc_failures > 0 {
                            warn!(crc_failures = stats.crc_failures, "MAVLink CRC failures in datagram");
                        }
                    }
                    Err(error) => {
                        warn!(%error, "MAVLink socket receive failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
}

/// Folds decoded messages into the shared cache. Kept synchronous and
/// lock-scoped: the lock is never held across an await.
fn apply_messages(
    state: &Arc<Mutex<LatestAviate>>,
    messages: &[(u8, AviateMessage)],
    crc_failures: u32,
    unknown_ids: u32,
) {
    let now = Instant::now();
    let Ok(mut latest) = state.lock() else {
        return;
    };
    latest.crc_failures = latest.crc_failures.wrapping_add(u64::from(crc_failures));
    latest.unknown_ids = latest.unknown_ids.wrapping_add(u64::from(unknown_ids));
    for &(sysid, message) in messages {
        latest.decoded = latest.decoded.wrapping_add(1);
        // Lock onto the first system id that delivers an estimate;
        // heartbeats alone don't lock (other GCS peers heartbeat too).
        let is_estimate = !matches!(message, AviateMessage::Heartbeat);
        match latest.locked_sysid {
            None if is_estimate => latest.locked_sysid = Some(sysid),
            Some(locked) if locked != sysid => continue,
            None => continue,
            Some(_) => {}
        }
        match message {
            AviateMessage::Heartbeat => latest.last_heartbeat = Some(now),
            AviateMessage::AttitudeQuaternion {
                time_boot_ms: _,
                quat_wxyz,
                rates_rps,
            } => {
                latest.attitude = Some(AttitudeUpdate {
                    quat_wxyz,
                    rates_rps,
                    received_at: now,
                });
            }
            AviateMessage::LocalPositionNed {
                time_boot_ms,
                pos_ned_m,
                vel_ned_mps,
            } => {
                latest.kinematics = Some(KinematicsUpdate {
                    pos_ned_m,
                    vel_ned_mps,
                    time_boot_ms,
                    received_at: now,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;

//! The UDP MAVLink link task: receives Aviate telemetry, caches the
//! latest estimate, and (in router mode) keeps this endpoint registered
//! with GCS heartbeats.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use pilotage_adapter_api::{MeasurementStamp, SourceIncarnation};

use crate::error::AviateAdapterError;
use crate::mavlink::{AviateMessage, FrameSource, encode_gcs_heartbeat, parse_datagram};

mod measurement;
use measurement::{next_attitude_stamp, next_kinematics_stamp};

/// Whether an unstamped MAVLink boot-clock regression may use the simulator
/// reset heuristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResetPolicy {
    /// Never infer a reboot from replayable MAVLink telemetry.
    #[default]
    Conservative,
    /// Permit a quarantined, silence-and-dwell-qualified simulator reset.
    SimulatorHeuristic,
}

/// Where the Aviate MAVLink telemetry is reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    /// The MAVLink GCS endpoint. If this port is free the adapter binds
    /// it directly (the FC pushes datagrams straight at it); if it is
    /// taken (a MAVLink router owns it), the adapter binds an ephemeral
    /// port and registers with the router via 1 Hz GCS heartbeats.
    pub endpoint: SocketAddr,
    /// Expected MAVLink vehicle system id.
    pub system_id: u8,
    /// Expected MAVLink producer component id.
    pub component_id: u8,
    /// Logical source id published above the MAVLink transport.
    pub source_id: u64,
    /// Policy for a boot-clock regression without a source boot UUID.
    pub reset_policy: ResetPolicy,
    /// Largest source-clock lag admitted when a second measurement group
    /// first joins or advances behind the epoch high-water mark.
    ///
    /// Zero (the fail-safe default) admits no inter-group lag at all: on
    /// an interleaved multi-rate stream the slower group is rejected as
    /// reordered every time the faster group advances the high-water
    /// mark. Every real deployment must set a budget derived from its
    /// source's publication rates — [`LinkConfig::simulator`] shows the
    /// Aviate-derived example — and the link warns at startup when the
    /// budget is zero so the rejection is loud, never silent.
    pub maximum_inter_group_skew_ms: u32,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            endpoint: SocketAddr::from(([127, 0, 0, 1], 14550)),
            system_id: 1,
            component_id: 1,
            source_id: 1,
            reset_policy: ResetPolicy::Conservative,
            maximum_inter_group_skew_ms: 0,
        }
    }
}

impl LinkConfig {
    /// Simulator profile with the bounded boot-clock reset heuristic enabled.
    #[must_use]
    pub fn simulator() -> Self {
        Self {
            reset_policy: ResetPolicy::SimulatorHeuristic,
            maximum_inter_group_skew_ms: 300,
            ..Self::default()
        }
    }
}

/// Latest state received from the FC, with receive stamps so staleness
/// can propagate to consumers (ADR-0018: loss of data marks groups
/// stale rather than freezing them).
#[derive(Debug)]
pub struct LatestAviate {
    /// Configured MAVLink vehicle system id.
    pub system_id: u8,
    /// Configured MAVLink producer component id.
    pub component_id: u8,
    /// Logical source id published above MAVLink.
    pub source_id: u64,
    /// Opaque identity of this adapter attachment.
    pub source_incarnation: SourceIncarnation,
    /// Reset inference policy for this source.
    pub reset_policy: ResetPolicy,
    /// Configured epoch-wide inter-group source-clock lag bound.
    pub maximum_inter_group_skew_ms: u32,
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
    /// Acquisition-clock generation for this FC connection.
    pub source_epoch: u32,
    /// Highest current-epoch FC boot timestamp accepted across groups.
    pub last_source_time_ms: Option<u32>,
    /// Receive time of the last accepted new group measurement.
    pub last_accepted_at: Option<Instant>,
    /// Candidate low timestamps awaiting simulator-only confirmation.
    pub(crate) pending_reset: Option<measurement::ResetCandidate>,
    /// Duplicate group measurements rejected before entering the cache.
    pub duplicate_measurements: u64,
    /// Older group measurements rejected before entering the cache.
    pub reordered_measurements: u64,
    /// Confirmed reboot or acquisition-clock-wrap transitions.
    pub source_resets: u64,
    /// Low-clock reset candidates quarantined for confirmation.
    pub suspected_resets: u64,
    /// Frames rejected because their system or component id was not selected.
    pub wrong_sources: u64,
}

impl Default for LatestAviate {
    fn default() -> Self {
        Self {
            system_id: 1,
            component_id: 1,
            source_id: 1,
            source_incarnation: SourceIncarnation::new([0; 16]),
            reset_policy: ResetPolicy::Conservative,
            maximum_inter_group_skew_ms: 0,
            attitude: None,
            kinematics: None,
            last_heartbeat: None,
            decoded: 0,
            crc_failures: 0,
            unknown_ids: 0,
            source_epoch: 1,
            last_source_time_ms: None,
            last_accepted_at: None,
            pending_reset: None,
            duplicate_measurements: 0,
            reordered_measurements: 0,
            source_resets: 0,
            suspected_resets: 0,
            wrong_sources: 0,
        }
    }
}

impl LatestAviate {
    fn for_source(config: LinkConfig, source_incarnation: SourceIncarnation) -> Self {
        Self {
            system_id: config.system_id,
            component_id: config.component_id,
            source_id: config.source_id,
            source_incarnation,
            reset_policy: config.reset_policy,
            maximum_inter_group_skew_ms: config.maximum_inter_group_skew_ms,
            source_epoch: 1,
            ..Self::default()
        }
    }
}

/// One attitude update with its receive stamp.
#[derive(Debug, Clone, Copy)]
pub struct AttitudeUpdate {
    /// Quaternion (w, x, y, z), body FRD → world NED.
    pub quat_wxyz: [f32; 4],
    /// Body rates (p, q, r) rad/s.
    pub rates_rps: [f32; 3],
    /// Milliseconds since FC boot.
    pub time_boot_ms: u32,
    /// Identity and acquisition stamp for this group update.
    pub stamp: MeasurementStamp,
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
    /// Identity and acquisition stamp for this group update.
    pub stamp: MeasurementStamp,
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
    pub async fn start(
        config: LinkConfig,
        source_incarnation: SourceIncarnation,
    ) -> Result<Self, AviateAdapterError> {
        if config.maximum_inter_group_skew_ms == 0 {
            warn!(
                "inter-group skew budget is zero: the slower of any \
                 interleaved measurement groups will be rejected as \
                 reordered until a rate-derived budget is configured"
            );
        }
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
        let state = Arc::new(Mutex::new(LatestAviate::for_source(
            config,
            source_incarnation,
        )));
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
    messages: &[(FrameSource, AviateMessage)],
    crc_failures: u32,
    unknown_ids: u32,
) {
    apply_messages_at(state, messages, crc_failures, unknown_ids, Instant::now());
}

fn apply_messages_at(
    state: &Arc<Mutex<LatestAviate>>,
    messages: &[(FrameSource, AviateMessage)],
    crc_failures: u32,
    unknown_ids: u32,
    now: Instant,
) {
    let Ok(mut latest) = state.lock() else {
        return;
    };
    latest.crc_failures = latest.crc_failures.wrapping_add(u64::from(crc_failures));
    latest.unknown_ids = latest.unknown_ids.wrapping_add(u64::from(unknown_ids));
    for &(source, message) in messages {
        latest.decoded = latest.decoded.wrapping_add(1);
        if source.system_id != latest.system_id || source.component_id != latest.component_id {
            latest.wrong_sources = latest.wrong_sources.wrapping_add(1);
            continue;
        }
        match message {
            AviateMessage::Heartbeat { .. } => latest.last_heartbeat = Some(now),
            AviateMessage::CommandAck { .. } => {}
            AviateMessage::AttitudeQuaternion {
                time_boot_ms,
                quat_wxyz,
                rates_rps,
            } => {
                if let Some(stamp) = next_attitude_stamp(&mut latest, time_boot_ms, now) {
                    latest.attitude = Some(AttitudeUpdate {
                        quat_wxyz,
                        rates_rps,
                        time_boot_ms,
                        stamp,
                        received_at: now,
                    });
                }
            }
            AviateMessage::LocalPositionNed {
                time_boot_ms,
                pos_ned_m,
                vel_ned_mps,
            } => {
                if let Some(stamp) = next_kinematics_stamp(&mut latest, time_boot_ms, now) {
                    latest.kinematics = Some(KinematicsUpdate {
                        pos_ned_m,
                        vel_ned_mps,
                        time_boot_ms,
                        stamp,
                        received_at: now,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

//! The UDP MAVLink link task: receives Aviate telemetry, caches the
//! latest estimate, and (in router mode) keeps this endpoint registered
//! with GCS heartbeats.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use pilotage_adapter_api::{MeasurementClock, MeasurementStamp};

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
    /// Acquisition-clock generation for this FC connection.
    pub source_epoch: u32,
    /// Highest current-epoch FC boot timestamp observed across groups.
    pub last_source_time_ms: Option<u32>,
    /// Candidate low timestamps awaiting confirmation of an FC reboot.
    pub(crate) pending_reset: Option<ResetCandidate>,
    /// Duplicate group measurements rejected before entering the cache.
    pub duplicate_measurements: u64,
    /// Older group measurements rejected before entering the cache.
    pub reordered_measurements: u64,
    /// Confirmed reboot or acquisition-clock-wrap transitions.
    pub source_resets: u64,
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

const RESET_PREVIOUS_MIN_MS: u32 = 30_000;
const RESET_CANDIDATE_MAX_MS: u32 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResetCandidate {
    latest_time_ms: u32,
    groups: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeasurementGroup {
    Attitude,
    Kinematics,
}

impl MeasurementGroup {
    const fn bit(self) -> u8 {
        match self {
            Self::Attitude => 1,
            Self::Kinematics => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeObservation {
    CurrentEpoch,
    PendingReset,
    NewEpoch,
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

fn serial_is_newer(candidate: u32, current: u32) -> bool {
    let distance = candidate.wrapping_sub(current);
    distance != 0 && distance < (1_u32 << 31)
}

fn begin_source_epoch(latest: &mut LatestAviate, time_boot_ms: u32) {
    latest.source_epoch = latest.source_epoch.wrapping_add(1);
    latest.last_source_time_ms = Some(time_boot_ms);
    latest.pending_reset = None;
    latest.attitude = None;
    latest.kinematics = None;
    latest.source_resets = latest.source_resets.wrapping_add(1);
    warn!(
        source_epoch = latest.source_epoch,
        time_boot_ms, "MAVLink acquisition clock entered a new epoch"
    );
}

fn observe_source_time(
    latest: &mut LatestAviate,
    group: MeasurementGroup,
    time_boot_ms: u32,
) -> TimeObservation {
    let Some(current) = latest.last_source_time_ms else {
        latest.last_source_time_ms = Some(time_boot_ms);
        return TimeObservation::CurrentEpoch;
    };
    if serial_is_newer(time_boot_ms, current) {
        if time_boot_ms < current {
            begin_source_epoch(latest, time_boot_ms);
            return TimeObservation::NewEpoch;
        }
        latest.last_source_time_ms = Some(time_boot_ms);
        latest.pending_reset = None;
        return TimeObservation::CurrentEpoch;
    }
    if time_boot_ms == current
        || current < RESET_PREVIOUS_MIN_MS
        || time_boot_ms > RESET_CANDIDATE_MAX_MS
    {
        latest.pending_reset = None;
        return TimeObservation::CurrentEpoch;
    }

    let bit = group.bit();
    match latest.pending_reset {
        Some(candidate)
            if serial_is_newer(time_boot_ms, candidate.latest_time_ms)
                || candidate.groups & bit == 0 =>
        {
            begin_source_epoch(latest, time_boot_ms);
            TimeObservation::NewEpoch
        }
        Some(_) => TimeObservation::PendingReset,
        None => {
            latest.pending_reset = Some(ResetCandidate {
                latest_time_ms: time_boot_ms,
                groups: bit,
            });
            TimeObservation::PendingReset
        }
    }
}

fn next_attitude_stamp(
    latest: &mut LatestAviate,
    sysid: u8,
    time_boot_ms: u32,
) -> Option<MeasurementStamp> {
    if observe_source_time(latest, MeasurementGroup::Attitude, time_boot_ms)
        == TimeObservation::PendingReset
    {
        return None;
    }
    next_group_stamp(
        latest
            .attitude
            .map(|update| (update.time_boot_ms, update.stamp)),
        latest,
        sysid,
        time_boot_ms,
    )
}

fn next_kinematics_stamp(
    latest: &mut LatestAviate,
    sysid: u8,
    time_boot_ms: u32,
) -> Option<MeasurementStamp> {
    if observe_source_time(latest, MeasurementGroup::Kinematics, time_boot_ms)
        == TimeObservation::PendingReset
    {
        return None;
    }
    next_group_stamp(
        latest
            .kinematics
            .map(|update| (update.time_boot_ms, update.stamp)),
        latest,
        sysid,
        time_boot_ms,
    )
}

fn next_group_stamp(
    current: Option<(u32, MeasurementStamp)>,
    latest: &mut LatestAviate,
    sysid: u8,
    time_boot_ms: u32,
) -> Option<MeasurementStamp> {
    let sequence = match current {
        None => 0,
        Some((current_time, _)) if current_time == time_boot_ms => {
            latest.duplicate_measurements = latest.duplicate_measurements.wrapping_add(1);
            return None;
        }
        Some((current_time, stamp)) if serial_is_newer(time_boot_ms, current_time) => {
            stamp.sequence.wrapping_add(1)
        }
        Some(_) => {
            latest.reordered_measurements = latest.reordered_measurements.wrapping_add(1);
            return None;
        }
    };
    Some(MeasurementStamp {
        source_id: u64::from(sysid),
        source_epoch: latest.source_epoch,
        sequence,
        acquired_at_ns: u64::from(time_boot_ms).wrapping_mul(1_000_000),
        clock: MeasurementClock::VehicleBoot,
    })
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
        let is_estimate = !matches!(
            message,
            AviateMessage::Heartbeat { .. } | AviateMessage::CommandAck { .. }
        );
        match latest.locked_sysid {
            None if is_estimate => {
                latest.locked_sysid = Some(sysid);
                latest.source_epoch = 1;
            }
            Some(locked) if locked != sysid => continue,
            None => continue,
            Some(_) => {}
        }
        match message {
            AviateMessage::Heartbeat { .. } => latest.last_heartbeat = Some(now),
            AviateMessage::CommandAck { .. } => {}
            AviateMessage::AttitudeQuaternion {
                time_boot_ms,
                quat_wxyz,
                rates_rps,
            } => {
                if let Some(stamp) = next_attitude_stamp(&mut latest, sysid, time_boot_ms) {
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
                if let Some(stamp) = next_kinematics_stamp(&mut latest, sysid, time_boot_ms) {
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

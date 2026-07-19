//! The UDP MAVLink link task: receives FC telemetry, caches the
//! latest estimate, and (in router mode) keeps this endpoint registered
//! with GCS heartbeats.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use pilotage_adapter_api::{MeasurementStamp, SourceIncarnation};

use crate::codec::{encode_gcs_heartbeat, parse_datagram};

mod apply;
mod error;
pub mod estimator;
pub mod measurement;
use apply::apply_messages;
pub use apply::apply_messages_at;
pub use error::LinkError;
use estimator::EstimatorStatusUpdate;

/// Which message carries the estimator authorization for cached numeric
/// groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthorizationSource {
    /// Aviate's private lossless estimator status (msg 20000): the FC
    /// emits a status for every numeric millisecond, so authorization
    /// requires an exact source-time match.
    #[default]
    AviatePrivate,
    /// The standard ESTIMATOR_STATUS (msg 230), as PX4 streams it: the
    /// status arrives at its own (slower) rate, so a numeric group is
    /// authorized by the most recent status within a bounded lag.
    StandardEstimatorStatus,
}

/// MAV_CMD_SET_MESSAGE_INTERVAL.
const SET_MESSAGE_INTERVAL: u16 = 511;

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

/// Where the FC MAVLink telemetry is reachable.
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
    /// Which message carries estimator authorization for this source.
    pub authorization_source: AuthorizationSource,
    /// Longest a standard-status authorization stays current for later
    /// numeric groups (StandardEstimatorStatus only). Must cover the
    /// configured status interval with margin, and must not exceed the
    /// display's status-to-numeric pairing budget or the panels flag
    /// samples this link still authorizes.
    pub standard_status_max_lag_ms: u32,
    /// Ceiling on a low boot clock that may seed a simulator reset
    /// candidate. Must exceed the FC's worst-case boot-to-streaming
    /// time: a rebooted FC whose clock is already above this ceiling is
    /// rejected as reordered forever instead of starting a new epoch.
    pub reset_candidate_max_ms: u32,
    /// Where periodic message-interval requests are sent, when the
    /// source needs them (PX4's GCS instance streams sparse defaults).
    /// They MUST originate from the link's own socket: the instance
    /// retargets its stream to whichever peer last spoke to it, so a
    /// request from any other socket steals the stream.
    pub stream_command_target: Option<SocketAddr>,
    /// (message id, interval µs) pairs requested at the heartbeat tick.
    pub stream_interval_requests: &'static [(u32, u32)],
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
            authorization_source: AuthorizationSource::AviatePrivate,
            standard_status_max_lag_ms: estimator::DEFAULT_STANDARD_STATUS_MAX_LAG_MS,
            reset_candidate_max_ms: measurement::DEFAULT_RESET_CANDIDATE_MAX_MS,
            stream_command_target: None,
            stream_interval_requests: &[],
            maximum_inter_group_skew_ms: 0,
        }
    }
}

impl LinkConfig {
    /// Physical-vehicle profile: the conservative reset policy (never
    /// infer a reboot from replayable telemetry) and the fail-safe zero
    /// inter-group skew budget. Every real deployment must set a budget
    /// derived from its link characteristics.
    #[must_use]
    pub fn physical() -> Self {
        Self::default()
    }

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
pub struct LinkState {
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
    /// Which message carries estimator authorization for this source.
    pub authorization_source: AuthorizationSource,
    /// Configured standard-status authorization lag ceiling.
    pub standard_status_max_lag_ms: u32,
    /// Configured ceiling on a reset-candidate boot clock.
    pub reset_candidate_max_ms: u32,
    /// Configured epoch-wide inter-group source-clock lag bound.
    pub maximum_inter_group_skew_ms: u32,
    /// Latest attitude estimate: quaternion (w,x,y,z), body rates,
    /// FC boot time, receive stamp.
    pub attitude: Option<AttitudeUpdate>,
    /// Latest NED kinematics: position, velocity, FC boot time, receive
    /// stamp.
    pub kinematics: Option<KinematicsUpdate>,
    /// Latest accepted lossless estimator authorization report.
    pub estimator_status: Option<EstimatorStatusUpdate>,
    /// Receive stamp of the last FC heartbeat.
    pub last_heartbeat: Option<Instant>,
    /// Whether the last FC heartbeat reported the vehicle armed.
    pub heartbeat_armed: Option<bool>,
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
    pub pending_reset: Option<measurement::ResetCandidate>,
    /// Duplicate group measurements rejected before entering the cache.
    pub duplicate_measurements: u64,
    /// Older group measurements rejected before entering the cache.
    pub reordered_measurements: u64,
    /// Malformed or unparseable private estimator-status reports that forced
    /// cached authorization closed.
    pub invalid_estimator_statuses: u64,
    /// Confirmed reboot or acquisition-clock-wrap transitions.
    pub source_resets: u64,
    /// Low-clock reset candidates quarantined for confirmation.
    pub suspected_resets: u64,
    /// Frames rejected because their system or component id was not selected.
    pub wrong_sources: u64,
}

impl Default for LinkState {
    fn default() -> Self {
        Self {
            system_id: 1,
            component_id: 1,
            source_id: 1,
            source_incarnation: SourceIncarnation::new([0; 16]),
            reset_policy: ResetPolicy::Conservative,
            authorization_source: AuthorizationSource::AviatePrivate,
            standard_status_max_lag_ms: estimator::DEFAULT_STANDARD_STATUS_MAX_LAG_MS,
            reset_candidate_max_ms: measurement::DEFAULT_RESET_CANDIDATE_MAX_MS,
            maximum_inter_group_skew_ms: 0,
            attitude: None,
            kinematics: None,
            estimator_status: None,
            last_heartbeat: None,
            heartbeat_armed: None,
            decoded: 0,
            crc_failures: 0,
            unknown_ids: 0,
            source_epoch: 1,
            last_source_time_ms: None,
            last_accepted_at: None,
            pending_reset: None,
            duplicate_measurements: 0,
            reordered_measurements: 0,
            invalid_estimator_statuses: 0,
            source_resets: 0,
            suspected_resets: 0,
            wrong_sources: 0,
        }
    }
}

impl LinkState {
    fn for_source(config: LinkConfig, source_incarnation: SourceIncarnation) -> Self {
        Self {
            system_id: config.system_id,
            component_id: config.component_id,
            source_id: config.source_id,
            source_incarnation,
            reset_policy: config.reset_policy,
            authorization_source: config.authorization_source,
            standard_status_max_lag_ms: config.standard_status_max_lag_ms,
            reset_candidate_max_ms: config.reset_candidate_max_ms,
            maximum_inter_group_skew_ms: config.maximum_inter_group_skew_ms,
            source_epoch: 1,
            ..Self::default()
        }
    }

    /// Stamp of the last accepted estimator authorization report, if any.
    pub fn estimator_status_stamp(&self) -> Option<MeasurementStamp> {
        self.estimator_status.map(|status| status.stamp)
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
    /// Authorization bits retained for this numeric acquisition.
    pub valid_flags: u32,
    /// Canonical quality retained for this numeric acquisition.
    pub quality: u32,
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
    /// Authorization bits retained for this numeric acquisition.
    pub valid_flags: u32,
    /// Canonical quality retained for this numeric acquisition.
    pub quality: u32,
    /// When this update was received.
    pub received_at: Instant,
}

/// A running MAVLink link: the receive task plus the shared latest-state
/// cache the adapter samples from.
#[derive(Debug)]
pub struct MavlinkLink {
    state: Arc<Mutex<LinkState>>,
    task: JoinHandle<()>,
}

impl MavlinkLink {
    /// Binds the socket (direct or router mode) and spawns the receive
    /// task.
    ///
    /// # Errors
    ///
    /// Returns [`LinkError::Bind`] when no socket can be bound.
    pub async fn start(
        config: LinkConfig,
        source_incarnation: SourceIncarnation,
    ) -> Result<Self, LinkError> {
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
                    .map_err(|source| LinkError::Bind { source })?;
                (socket, true)
            }
        };
        info!(
            mode = if router_mode { "router" } else { "direct" },
            endpoint = %config.endpoint,
            "MAVLink link listening"
        );
        let state = Arc::new(Mutex::new(LinkState::for_source(
            config,
            source_incarnation,
        )));
        let task = tokio::spawn(run_link(socket, config, router_mode, state.clone()));
        Ok(Self { state, task })
    }

    /// The shared latest-state cache.
    pub fn state(&self) -> Arc<Mutex<LinkState>> {
        self.state.clone()
    }
}

impl Drop for MavlinkLink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_link(
    socket: UdpSocket,
    config: LinkConfig,
    router_mode: bool,
    state: Arc<Mutex<LinkState>>,
) {
    let endpoint = config.endpoint;
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
                if let Some(target) = config.stream_command_target {
                    for &(message_id, interval_us) in config.stream_interval_requests {
                        let frame = crate::codec::encode_command_long(
                            seq,
                            SET_MESSAGE_INTERVAL,
                            [message_id as f32, interval_us as f32, 0.0, 0.0, 0.0, 0.0, 0.0],
                            config.system_id,
                            config.component_id,
                        );
                        seq = seq.wrapping_add(1);
                        if let Err(error) = socket.send_to(&frame, target).await {
                            warn!(%error, "stream interval request send failed");
                        }
                    }
                }
            }
            received = socket.recv_from(&mut buf) => {
                match received {
                    Ok((len, _from)) => {
                        messages.clear();
                        let stats = parse_datagram(buf.get(..len).unwrap_or(&[]), &mut messages);
                        apply_messages(
                            &state,
                            &messages,
                            stats.crc_failures,
                            stats.unknown_ids,
                        );
                        if stats.crc_failures > 0 {
                            warn!(crc_failures = stats.crc_failures, "MAVLink CRC failures in datagram");
                        }
                        if stats.invalid_estimator_status_frames > 0 {
                            error!(
                                invalid_frames = stats.invalid_estimator_status_frames,
                                "private estimator status frame failed validation"
                            );
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

#[cfg(test)]
mod tests;

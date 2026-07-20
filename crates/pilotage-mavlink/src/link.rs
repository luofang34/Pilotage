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
mod outbound;
mod updates;
use apply::apply_messages;
pub use apply::apply_messages_at;
pub use error::LinkError;
use estimator::EstimatorStatusUpdate;
pub use outbound::{GimbalRateDemand, OutboundCommand};
use outbound::{send_gimbal_rate, send_outbound_command};
pub use updates::{AttitudeUpdate, CommandAckReport, GimbalDeviceAttitude, KinematicsUpdate};

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
    /// Latest gimbal-device orientation report. Cached outside the
    /// estimate measurement discipline: it is payload-device status,
    /// never an input to vehicle state or control validation.
    pub gimbal_device: Option<GimbalDeviceAttitude>,
    /// Latest command acknowledgement, kept so uplink senders can
    /// surface a typed denial instead of a silently dead command path.
    pub last_command_ack: Option<CommandAckReport>,
    /// Latest gimbal CONFIGURE acknowledgement, tracked apart from
    /// `last_command_ack` so unrelated acks cannot bury a claim denial
    /// while the FC silently ignores gimbal demands.
    pub gimbal_configure_ack: Option<CommandAckReport>,
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
            gimbal_device: None,
            last_command_ack: None,
            gimbal_configure_ack: None,
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

/// A running MAVLink link: the receive task plus the shared latest-state
/// cache the adapter samples from.
#[derive(Debug)]
pub struct MavlinkLink {
    state: Arc<Mutex<LinkState>>,
    task: JoinHandle<()>,
    commands: tokio::sync::mpsc::Sender<OutboundCommand>,
    gimbal_rates: Option<tokio::sync::watch::Sender<Option<GimbalRateDemand>>>,
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
        let (commands, command_rx) = tokio::sync::mpsc::channel(16);
        let (rate_tx, rate_rx) = tokio::sync::watch::channel(None);
        let task = tokio::spawn(run_link(
            socket,
            config,
            router_mode,
            state.clone(),
            command_rx,
            rate_rx,
        ));
        Ok(Self {
            state,
            task,
            commands,
            gimbal_rates: Some(rate_tx),
        })
    }

    /// The shared latest-state cache.
    pub fn state(&self) -> Arc<Mutex<LinkState>> {
        self.state.clone()
    }

    /// The ordered command lane toward the stream-command target. The
    /// FC's GCS instance retargets its stream to whichever peer last
    /// spoke, so uplink traffic for that instance must ride the link's
    /// own socket — and the link task assigns the socket's single
    /// MAVLink sequence, so callers never encode frames themselves.
    pub fn command_sender(&self) -> tokio::sync::mpsc::Sender<OutboundCommand> {
        self.commands.clone()
    }

    /// Takes the latest-value gimbal rate lane (single producer). Each
    /// publication replaces the previous demand; the link task encodes
    /// only the newest, so stale demands coalesce away under
    /// backpressure instead of queueing behind fresh ones.
    pub fn take_gimbal_rate_sender(
        &mut self,
    ) -> Option<tokio::sync::watch::Sender<Option<GimbalRateDemand>>> {
        self.gimbal_rates.take()
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
    mut command_rx: tokio::sync::mpsc::Receiver<OutboundCommand>,
    mut rate_rx: tokio::sync::watch::Receiver<Option<GimbalRateDemand>>,
) {
    let endpoint = config.endpoint;
    let mut buf = vec![0u8; 2048];
    let mut messages = Vec::with_capacity(8);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    let mut seq: u8 = 0;
    let mut rates_open = true;
    loop {
        tokio::select! {
            // Biased: the command lane is polled before the rate lane so a
            // queued CONFIGURE always reaches the wire before the rate
            // setpoint enqueued after it. Without this, an unbiased select
            // could transmit the first rate demand ahead of the claim and
            // PX4 would drop it as non-primary (claim-before-first-setpoint).
            biased;
            command = command_rx.recv() => {
                send_outbound_command(&socket, &mut seq, config.stream_command_target, command).await;
            }
            changed = rate_rx.changed(), if rates_open => {
                if changed.is_err() {
                    // The demand producer dropped; stop polling the lane.
                    rates_open = false;
                } else {
                    let demand = *rate_rx.borrow_and_update();
                    send_gimbal_rate(&socket, &mut seq, config.stream_command_target, demand).await;
                }
            }
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
                        fold_datagram(buf.get(..len).unwrap_or(&[]), &state, &mut messages);
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

/// Parses one datagram into the reusable message buffer, folds it into
/// the shared cache, and surfaces integrity failures loudly.
fn fold_datagram(
    bytes: &[u8],
    state: &Arc<Mutex<LinkState>>,
    messages: &mut Vec<(crate::codec::FrameSource, crate::codec::FcMessage)>,
) {
    messages.clear();
    let stats = parse_datagram(bytes, messages);
    apply_messages(state, messages, stats.crc_failures, stats.unknown_ids);
    if stats.crc_failures > 0 {
        warn!(
            crc_failures = stats.crc_failures,
            "MAVLink CRC failures in datagram"
        );
    }
    if stats.invalid_estimator_status_frames > 0 {
        error!(
            invalid_frames = stats.invalid_estimator_status_frames,
            "private estimator status frame failed validation"
        );
    }
}

#[cfg(test)]
mod arbiter_tests;
#[cfg(test)]
mod tests;

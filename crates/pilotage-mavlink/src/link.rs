//! The UDP MAVLink link: receives FC telemetry into a shared
//! latest-state cache, and carries this GCS's commands and setpoints
//! from the same socket.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use pilotage_adapter_api::{MeasurementStamp, SourceIncarnation};

mod apply;
mod config;
mod error;
pub mod estimator;
mod fc_reports;
mod fence;
mod gimbal;
pub use gimbal::{GimbalManagerReport, GimbalReports};
pub mod measurement;
mod outbound;
pub use fence::{CameraWriteRequirement, ManagerWriteRequirement, SendFence};
mod task;
mod updates;
pub use apply::apply_messages_at;
pub use config::{AuthorizationSource, CommandTarget, LinkConfig, ResetPolicy};
pub use error::LinkError;
pub use estimate_fix::estimate_geodetic_fix;
use estimator::EstimatorStatusUpdate;
pub use fc_reports::{FcReports, ParamReport, STATUS_TEXT_HISTORY, StatusTextReport};
pub use magnetic_variation::{fc_magnetic_variation, fc_variation_source};
use outbound::OutboundLanes;
pub use outbound::{
    AttitudeThrustDemand, BodyVelocityDemand, GimbalRateDemand, OffboardSetpoint, OutboundCommand,
    OutboundParam, ParamRequest,
};
use task::run_link;
pub use updates::{
    AttitudeUpdate, BaroUpdate, BatteryReport, CommandAckReport, GeomagneticReferenceUpdate,
    GimbalDeviceAttitude, GnssFixUpdate, KinematicsUpdate, SimTruthUpdate, TruthOrigin,
};

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
    /// Latest accepted static-pressure acquisition.
    pub baro: Option<BaroUpdate>,
    /// Latest simulator ground-truth report (SITL only). Cached outside
    /// the estimate measurement discipline like the gimbal report.
    pub sim_truth: Option<SimTruthUpdate>,
    /// The receiver's own fix, when the flight controller reports one.
    pub gnss_fix: Option<GnssFixUpdate>,
    /// The flight controller's latest geomagnetic reference report.
    pub geomagnetic_reference: Option<GeomagneticReferenceUpdate>,
    /// When this link began: the clock a report is stamped against when
    /// its own timestamp names a clock nothing identifies.
    pub started_at: Instant,
    /// Origin the truth stream projects against, latched on its first fix.
    pub truth_origin: Option<TruthOrigin>,
    /// Discovered payload identities and feedback, separate from vehicle authorization.
    pub gimbals: GimbalReports,
    /// Latest report of battery instance 0. Cached outside the estimate measurement
    /// discipline: it is vehicle state, never an input to an estimate.
    pub battery: Option<BatteryReport>,
    /// Battery reports of instances other than 0, dropped.
    pub other_battery_reports: u64,
    /// Latest command acknowledgement, kept so uplink senders can
    /// surface a typed denial instead of a silently dead command path.
    pub last_command_ack: Option<CommandAckReport>,
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
    /// FC-owned reports: flight mode, per-command verdicts, status text.
    pub fc_reports: FcReports,
}

mod camera;
pub use camera::CameraReport;
mod defaults;
mod estimate_fix;
mod magnetic_variation;

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
    setpoints: Option<tokio::sync::watch::Sender<Option<OffboardSetpoint>>>,
    params: tokio::sync::mpsc::Sender<OutboundParam>,
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
        let (velocity_tx, velocity_rx) = tokio::sync::watch::channel(None);
        let (params, param_rx) = tokio::sync::mpsc::channel(16);
        let lanes = OutboundLanes {
            commands: command_rx,
            gimbal_rates: rate_rx,
            setpoints: velocity_rx,
            params: param_rx,
        };
        let task = tokio::spawn(run_link(socket, config, router_mode, state.clone(), lanes));
        Ok(Self {
            state,
            task,
            commands,
            gimbal_rates: Some(rate_tx),
            setpoints: Some(velocity_tx),
            params,
        })
    }

    /// The shared latest-state cache.
    pub fn state(&self) -> Arc<Mutex<LinkState>> {
        self.state.clone()
    }

    /// The ordered command lane toward the command target. The FC's GCS
    /// instance retargets its stream to whichever peer last spoke, so
    /// uplink traffic for that instance must ride the link's own socket —
    /// and the link task assigns the socket's single MAVLink sequence, so
    /// callers never encode frames themselves.
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

    /// Takes the latest-value offboard setpoint lane (single producer),
    /// with the same coalescing as the gimbal rate lane. Every publication
    /// is sent, so a producer streams at the rate it publishes.
    pub fn take_setpoint_sender(
        &mut self,
    ) -> Option<tokio::sync::watch::Sender<Option<OffboardSetpoint>>> {
        self.setpoints.take()
    }

    /// The ordered parameter-protocol lane. Reads are answered with a
    /// PARAM_VALUE that the cache keeps in [`FcReports::params`].
    pub fn param_sender(&self) -> tokio::sync::mpsc::Sender<OutboundParam> {
        self.params.clone()
    }
}

impl Drop for MavlinkLink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod arbiter_tests;
#[cfg(test)]
mod peer_tests;
#[cfg(test)]
mod tests;

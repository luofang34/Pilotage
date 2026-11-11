//! The UDP link: the socket, the receive task, and the outbound lanes
//! around the shared latest-state cache.

use std::sync::{Arc, Mutex};

use pilotage_adapter_api::SourceIncarnation;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::outbound::OutboundLanes;
use super::task::run_link;
use super::{
    GimbalRateDemand, LinkConfig, LinkError, LinkState, OffboardSetpoint, OutboundCommand,
    OutboundParam,
};

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
            web_time::Instant::now(),
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
    /// PARAM_VALUE that the cache keeps in [`crate::link::FcReports::params`].
    pub fn param_sender(&self) -> tokio::sync::mpsc::Sender<OutboundParam> {
        self.params.clone()
    }
}

impl Drop for MavlinkLink {
    fn drop(&mut self) {
        self.task.abort();
    }
}

//! Where the FC is reachable and which policies the link applies to its
//! telemetry.

use std::net::SocketAddr;

use super::{estimator, measurement};

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
    /// ArduPilot's EKF_STATUS_REPORT (msg 193). The report has no
    /// timestamp, so the link stamps it at the newest FC boot time it has
    /// accepted from any group when the report arrives, and a numeric
    /// group is then authorized within the same bounded lag as the
    /// standard status.
    ArduPilotEkfStatus,
}

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

/// Where frames from the link's own socket go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandTarget {
    /// The link sends no commands.
    #[default]
    None,
    /// A fixed FC endpoint, such as PX4's GCS instance.
    Fixed(SocketAddr),
    /// The address the selected FC system and component last sent from.
    /// An ArduPilot SITL `udpclient` serial port is a connected socket on
    /// an ephemeral port: it takes datagrams only from the address it
    /// sends to, and its own address is known only once it speaks. In
    /// this mode the link also sends the 1 Hz GCS heartbeat there, because
    /// nothing else speaks for this GCS on that connection.
    FcSender,
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
    /// numeric groups (StandardEstimatorStatus and ArduPilotEkfStatus).
    /// Must cover the configured status interval with margin, and must
    /// not exceed the display's status-to-numeric pairing budget or the
    /// panels flag samples this link still authorizes.
    pub standard_status_max_lag_ms: u32,
    /// Ceiling on a low boot clock that may seed a simulator reset
    /// candidate. Must exceed the FC's worst-case boot-to-streaming
    /// time: a rebooted FC whose clock is already above this ceiling is
    /// rejected as reordered forever instead of starting a new epoch.
    pub reset_candidate_max_ms: u32,
    /// Where commands, setpoints, and periodic message-interval requests
    /// are sent. They MUST originate from the link's own socket: an FC
    /// GCS instance retargets its stream to whichever peer last spoke to
    /// it, so a request from any other socket steals the stream.
    pub command_target: CommandTarget,
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
            command_target: CommandTarget::None,
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

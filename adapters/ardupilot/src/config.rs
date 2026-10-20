//! Typed ArduPilot adapter configuration.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use pilotage_mavlink::codec::{
    ATTITUDE_QUATERNION_ID, BATTERY_STATUS_ID, EKF_STATUS_REPORT_ID, GNSS_RAW_ID,
    LOCAL_POSITION_NED_ID,
};
use pilotage_mavlink::{AuthorizationSource, CommandTarget, LinkConfig};

/// ArduPilot operating profiles supported by this adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArduPilotProfile {
    /// ArduPilot SITL with the bounded simulator reset heuristic enabled.
    /// The adapter sets the parameter bits direct flight needs
    /// (`adapter/params.rs`).
    Simulation,
    /// A real vehicle: the conservative reset policy, and the adapter
    /// never writes a parameter. It reads the ones direct flight needs and
    /// refuses direct flight while one is wrong.
    Physical,
}

/// All policy and network configuration required to start the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArduPilotConfig {
    profile: ArduPilotProfile,
    /// Local UDP endpoint the FC sends MAVLink to. ArduPilot SITL reaches
    /// it with `--serial0 udpclient:<this address>`; the adapter answers
    /// the address the FC sends from, so the FC needs no fixed port.
    pub endpoint: SocketAddr,
    /// MAVLink system id of the vehicle.
    pub system_id: u8,
}

/// The EKF status interval: 10 Hz, so the 300 ms authorization lag below
/// covers two missed reports.
const EKF_STATUS_INTERVAL_US: u32 = 100_000;

impl ArduPilotConfig {
    /// The default configuration for an explicit profile: endpoint
    /// `127.0.0.1:14550`, system id 1.
    #[must_use]
    pub fn new(profile: ArduPilotProfile) -> Self {
        Self {
            profile,
            endpoint: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 14_550)),
            system_id: 1,
        }
    }

    /// The operating profile.
    #[must_use]
    pub(crate) fn profile(self) -> ArduPilotProfile {
        self.profile
    }

    pub(crate) fn link_config(self) -> LinkConfig {
        let mut config = match self.profile {
            ArduPilotProfile::Simulation => LinkConfig::simulator(),
            ArduPilotProfile::Physical => LinkConfig::physical(),
        };
        config.endpoint = self.endpoint;
        config.system_id = self.system_id;
        // The autopilot itself is component 1 (MAV_COMP_ID_AUTOPILOT1).
        config.component_id = 1;
        config.authorization_source = AuthorizationSource::ArduPilotEkfStatus;
        config.standard_status_max_lag_ms = 300;
        config.reset_candidate_max_ms = 60_000;
        config.command_target = CommandTarget::FcSender;
        // ArduPilot streams sparse defaults on a serial port nobody has
        // configured, so every lane the samples read is requested:
        // attitude and local position at 30 Hz, the receiver fix at 5 Hz,
        // the EKF status that authorizes them, and the battery at 1 Hz.
        config.stream_interval_requests = &[
            (ATTITUDE_QUATERNION_ID, 33_333),
            (LOCAL_POSITION_NED_ID, 33_333),
            (GNSS_RAW_ID, 200_000),
            (EKF_STATUS_REPORT_ID, EKF_STATUS_INTERVAL_US),
            (BATTERY_STATUS_ID, 1_000_000),
        ];
        config
    }
}

#[cfg(test)]
mod tests;

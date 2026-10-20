//! Typed ArduPilot adapter configuration.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use pilotage_mavlink::codec::{
    ATTITUDE_QUATERNION_ID, BATTERY_STATUS_ID, EKF_STATUS_REPORT_ID,
    GIMBAL_DEVICE_ATTITUDE_STATUS_ID, GNSS_RAW_ID, LOCAL_POSITION_NED_ID,
};
use pilotage_mavlink::{AuthorizationSource, CommandTarget, LinkConfig};

/// ArduPilot operating profiles supported by this adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArduPilotProfile {
    /// ArduPilot SITL with the bounded simulator reset heuristic enabled.
    Simulation,
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
    /// Whether to discover a MAVLink Gimbal Protocol v2 payload on this link.
    /// Discovery does not claim control or assume which axes the device supports.
    pub gimbal: bool,
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
            gimbal: false,
        }
    }

    /// Enables gimbal discovery and its command path without claiming control.
    #[must_use]
    pub fn with_gimbal(mut self, gimbal: bool) -> Self {
        self.gimbal = gimbal;
        self
    }

    pub(crate) fn link_config(self) -> LinkConfig {
        let mut config = match self.profile {
            ArduPilotProfile::Simulation => LinkConfig::simulator(),
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
        config.stream_interval_requests = stream_intervals(self.gimbal);
        config
    }
}

fn stream_intervals(gimbal: bool) -> &'static [(u32, u32)] {
    const FLIGHT: [(u32, u32); 5] = [
        (ATTITUDE_QUATERNION_ID, 33_333),
        (LOCAL_POSITION_NED_ID, 33_333),
        (GNSS_RAW_ID, 200_000),
        (EKF_STATUS_REPORT_ID, EKF_STATUS_INTERVAL_US),
        (BATTERY_STATUS_ID, 1_000_000),
    ];
    // ManagerControl requests manager information and ownership status itself.
    // Device feedback needs a continuous stream for the operator's camera view.
    const WITH_GIMBAL: [(u32, u32); 6] = [
        FLIGHT[0],
        FLIGHT[1],
        FLIGHT[2],
        FLIGHT[3],
        FLIGHT[4],
        (GIMBAL_DEVICE_ATTITUDE_STATUS_ID, 20_000),
    ];
    if gimbal { &WITH_GIMBAL } else { &FLIGHT }
}

#[cfg(test)]
mod tests;

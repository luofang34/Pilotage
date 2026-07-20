//! Typed PX4 adapter configuration.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use pilotage_mavlink::{AuthorizationSource, LinkConfig};

/// PX4 operating profiles supported by this adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Px4Profile {
    /// PX4 SITL with the bounded simulator reset heuristic enabled.
    Simulation,
}

/// All policy and network configuration required to start the PX4 adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Px4Config {
    profile: Px4Profile,
    /// Local endpoint receiving PX4 telemetry.
    pub telemetry_endpoint: SocketAddr,
    /// PX4 GCS endpoint receiving stream-interval commands.
    pub stream_command_endpoint: SocketAddr,
    /// PX4 onboard endpoint receiving offboard commands.
    pub command_endpoint: SocketAddr,
    /// Whether this vehicle carries a MAVLink Gimbal Protocol v2 gimbal.
    /// Off by default: a bare airframe has no gimbal, and advertising
    /// the `vehicle.gimbal` scope on one would let a client lease a
    /// payload the vehicle cannot point. `PILOTAGE_PX4_GIMBAL=1` (via
    /// the launcher) or [`Px4Config::with_gimbal`] enables it.
    pub gimbal: bool,
}

impl Px4Config {
    /// Builds the default endpoint set for an explicit PX4 profile. The
    /// gimbal capability is off; enable it with [`Px4Config::with_gimbal`].
    #[must_use]
    pub fn new(profile: Px4Profile) -> Self {
        Self {
            profile,
            telemetry_endpoint: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 14_550)),
            stream_command_endpoint: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 18_570)),
            command_endpoint: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 14_580)),
            gimbal: false,
        }
    }

    /// Declares that this vehicle carries a gimbal, so the adapter
    /// advertises the `vehicle.gimbal` scope and wires its command path.
    #[must_use]
    pub fn with_gimbal(mut self, gimbal: bool) -> Self {
        self.gimbal = gimbal;
        self
    }

    pub(crate) fn link_config(self) -> LinkConfig {
        let mut config = match self.profile {
            Px4Profile::Simulation => LinkConfig::simulator(),
        };
        config.endpoint = self.telemetry_endpoint;
        config.authorization_source = AuthorizationSource::StandardEstimatorStatus;
        config.standard_status_max_lag_ms = 300;
        config.reset_candidate_max_ms = 60_000;
        config.stream_command_target = Some(self.stream_command_endpoint);
        config.stream_interval_requests = &[(31, 33_333), (32, 33_333), (230, 100_000)];
        config
    }
}

#[cfg(test)]
mod tests {
    use pilotage_mavlink::ResetPolicy;

    use super::{Px4Config, Px4Profile};

    #[test]
    fn simulation_profile_enables_only_the_simulator_reset_policy() {
        let config = Px4Config::new(Px4Profile::Simulation).link_config();
        assert_eq!(config.reset_policy, ResetPolicy::SimulatorHeuristic);
    }
}

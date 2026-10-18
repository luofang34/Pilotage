//! Typed PX4 adapter configuration.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use pilotage_mavlink::{AuthorizationSource, LinkConfig};
use pilotage_protocol::VehicleClass;

/// PX4 operating profiles supported by this adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Px4Profile {
    /// PX4 SITL with the bounded simulator reset heuristic enabled.
    Simulation,
}

/// The airframe class PX4 flies, which decides how the adapter turns the
/// unified velocity intent into PX4 commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Px4Airframe {
    /// A multirotor: velocity setpoints in offboard mode.
    #[default]
    Multicopter,
    /// A conventional airplane: Position-mode sticks over MANUAL_CONTROL,
    /// with an automatic takeoff on arm.
    FixedWing,
}

impl Px4Airframe {
    /// The vehicle class the capability advertisement declares for this
    /// airframe. The airframe is the single source: the class a client
    /// picks its control scheme from always matches how the adapter flies.
    #[must_use]
    pub const fn vehicle_class(self) -> VehicleClass {
        match self {
            Self::Multicopter => VehicleClass::Multicopter,
            Self::FixedWing => VehicleClass::FixedWing,
        }
    }
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
    /// The MAVLink system id of the PX4 instance this adapter flies. PX4
    /// SITL instance `N` reports system id `N + 1`, so concurrent instances
    /// are told apart by this id, never by port alone.
    pub system_id: u8,
    /// The airframe class behind this PX4 instance.
    pub airframe: Px4Airframe,
    /// Whether this vehicle carries a MAVLink Gimbal Protocol v2 gimbal.
    /// Off by default: a bare airframe has no gimbal, and advertising
    /// the `vehicle.gimbal` scope on one would let a client lease a
    /// payload the vehicle cannot point. `PILOTAGE_PX4_GIMBAL=1` (via
    /// the launcher) or [`Px4Config::with_gimbal`] enables it.
    pub gimbal: bool,
    /// Acceptance fault injection: drop the gimbal link-loss stop instead of
    /// sending it, so the vehicle's own failsafe is the sole mechanism under
    /// test. Permitted ONLY under [`Px4Profile::Simulation`] — a real vehicle
    /// must never withhold its safe-state command — and enforced by
    /// [`Px4Config::with_gimbal_stop_dropped`].
    drop_gimbal_link_loss_stop: bool,
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
            system_id: 1,
            airframe: Px4Airframe::Multicopter,
            gimbal: false,
            drop_gimbal_link_loss_stop: false,
        }
    }

    /// Declares that this vehicle carries a gimbal, so the adapter
    /// advertises the `vehicle.gimbal` scope and wires its command path.
    #[must_use]
    pub fn with_gimbal(mut self, gimbal: bool) -> Self {
        self.gimbal = gimbal;
        self
    }

    /// Enables the gimbal link-loss-stop fault injection (acceptance testing of
    /// the vehicle's own failsafe). Enforced fail-safe: it takes effect ONLY
    /// under [`Px4Profile::Simulation`], so a real vehicle can never be
    /// configured to withhold its safe-state command.
    #[must_use]
    pub fn with_gimbal_stop_dropped(mut self, drop: bool) -> Self {
        self.drop_gimbal_link_loss_stop = drop && self.profile == Px4Profile::Simulation;
        self
    }

    /// Whether the gimbal link-loss stop is dropped (Simulation-only fault
    /// injection). Always `false` outside `Simulation`.
    #[must_use]
    pub(crate) fn drop_gimbal_link_loss_stop(&self) -> bool {
        self.drop_gimbal_link_loss_stop
    }

    pub(crate) fn link_config(self) -> LinkConfig {
        let mut config = match self.profile {
            Px4Profile::Simulation => LinkConfig::simulator(),
        };
        config.endpoint = self.telemetry_endpoint;
        config.system_id = self.system_id;
        config.authorization_source = AuthorizationSource::StandardEstimatorStatus;
        config.standard_status_max_lag_ms = 300;
        config.reset_candidate_max_ms = 60_000;
        config.stream_command_target = Some(self.stream_command_endpoint);
        // GPS_RAW_INT carries the estimate lane's position. Most
        // autopilots stream it by default, and a lane that draws a vehicle
        // should not rest on a default: 5 Hz is the rate a receiver solves
        // at, and asking costs one message at startup.
        // BATTERY_STATUS at 1 Hz: the battery changes slowly, and the
        // sample is withheld after three missed reports.
        // GIMBAL_DEVICE_ATTITUDE_STATUS at 50 Hz: PX4 streams it at 5 Hz by
        // default, and a camera view steered by the stick follows the gimbal
        // only as often as it hears where it points. PX4 sends no faster than
        // its gimbal reports, so the request asks only for what it measures.
        config.stream_interval_requests = &[
            (24, 200_000),
            (31, 33_333),
            (32, 33_333),
            (147, 1_000_000),
            (230, 100_000),
            (285, 20_000),
        ];
        config
    }
}

#[cfg(test)]
mod tests {
    use pilotage_mavlink::ResetPolicy;

    use super::{Px4Airframe, Px4Config, Px4Profile};

    #[test]
    fn the_battery_report_is_requested_at_one_hertz() {
        let config = Px4Config::new(Px4Profile::Simulation).link_config();
        assert!(config.stream_interval_requests.contains(&(147, 1_000_000)));
    }

    #[test]
    fn the_gimbal_attitude_is_requested_at_fifty_hertz() {
        let config = Px4Config::new(Px4Profile::Simulation).link_config();
        let gimbal = pilotage_mavlink::codec::GIMBAL_DEVICE_ATTITUDE_STATUS_ID;
        assert!(config.stream_interval_requests.contains(&(gimbal, 20_000)));
    }

    #[test]
    fn the_advertised_class_follows_the_airframe() {
        use pilotage_protocol::VehicleClass;
        assert_eq!(
            Px4Airframe::Multicopter.vehicle_class(),
            VehicleClass::Multicopter
        );
        assert_eq!(
            Px4Airframe::FixedWing.vehicle_class(),
            VehicleClass::FixedWing
        );
        assert_eq!(
            Px4Config::new(Px4Profile::Simulation).airframe,
            Px4Airframe::Multicopter
        );
    }

    #[test]
    fn the_link_listens_to_the_configured_system_only() {
        let mut px4 = Px4Config::new(Px4Profile::Simulation);
        assert_eq!(px4.link_config().system_id, 1);
        px4.system_id = 7;
        assert_eq!(px4.link_config().system_id, 7);
    }

    #[test]
    fn simulation_profile_enables_only_the_simulator_reset_policy() {
        let config = Px4Config::new(Px4Profile::Simulation).link_config();
        assert_eq!(config.reset_policy, ResetPolicy::SimulatorHeuristic);
    }
}

//! What the embedding host gives the core once, at construction.

use pilotage_adapter_aviate::EmbeddedIncarnations;
use pilotage_control_feel::ValidatedFlightFeelProfile;
use pilotage_mavlink::link::LinkConfig;
use pilotage_mission::MissionConfig;
use pilotage_protocol::VehicleId;

/// The configuration of one [`crate::HostCore`], by value.
#[derive(Debug, Clone)]
pub struct HostCoreConfig {
    /// The vehicle the core hosts.
    pub vehicle: VehicleId,
    /// The flight controller's MAVLink identity and link policy.
    pub link: LinkConfig,
    /// The control-feel law the adapter's uplink shapes commands with.
    pub control_feel: ValidatedFlightFeelProfile,
    /// The identities the adapter stamps its sources with.
    pub incarnations: EmbeddedIncarnations,
    /// The mission configuration, including the vehicle class and the
    /// ownship source.
    pub mission: MissionConfig,
    /// The civil time, Unix seconds, at which the magnetic field is
    /// evaluated for the whole run. The core reads no civil clock.
    pub magnetic_epoch_unix_s: i64,
}

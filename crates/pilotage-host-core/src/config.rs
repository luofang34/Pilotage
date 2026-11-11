//! What the embedding host gives the core once, at construction.

use pilotage_adapter_aviate::{AviateAirframe, EmbeddedIncarnations};
use pilotage_mavlink::link::LinkConfig;
use pilotage_mission::MissionConfig;

/// The configuration of one [`crate::HostCore`], by value.
#[derive(Debug, Clone)]
pub struct HostCoreConfig {
    /// The flight controller's MAVLink identity and link policy.
    pub link: LinkConfig,
    /// The airframe the flight controller flies: a multirotor with the
    /// control-feel law its commands are shaped by, or a fixed wing with
    /// the envelope its demands are bounded by.
    pub airframe: AviateAirframe,
    /// The height of the flight controller's local origin above the WGS84
    /// ellipsoid, metres: the same number the embedding host gives the
    /// flight controller as its origin. Without it, a heading-and-altitude
    /// demand on an ellipsoidal altitude is refused.
    pub fc_origin_height_m: Option<f64>,
    /// The identities the adapter stamps its sources with.
    pub incarnations: EmbeddedIncarnations,
    /// The mission configuration, including the vehicle class and the
    /// ownship source.
    pub mission: MissionConfig,
    /// The civil time, Unix seconds, at which the magnetic field is
    /// evaluated for the whole run. The core reads no civil clock.
    pub magnetic_epoch_unix_s: i64,
}

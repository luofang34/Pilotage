//! Where the world's heights meet the shared world model's geodesy.
//!
//! The world states heights above mean sea level: Terrarium terrain, site
//! elevations, and a simulator's altitude all are. The shared world model
//! (`world-core`) takes heights above the WGS-84 ellipsoid and has no
//! geoid model yet. The simulated world therefore uses a flat simulator
//! geoid ([`crate::WORLD_GEOID`]) with zero separation: an MSL height is
//! its ellipsoidal height. A real geoid would add tens of metres everywhere
//! alike, which moves a vehicle against its own terrain not at all, and the
//! magnetic field by far less than the model's uncertainty.

pub use world_core::{LocalTangentPlane, Ned, PositionError};

/// A WGS-84 position from a world (MSL) height, under the flat simulator
/// geoid.
pub fn world_position(
    latitude_deg: f64,
    longitude_deg: f64,
    height_msl_m: f64,
) -> world_core::GeodeticPosition {
    world_core::GeodeticPosition::wgs84(latitude_deg, longitude_deg, height_msl_m)
}

/// The local north-east-down frame tangent at a world (MSL) position.
///
/// # Errors
///
/// Returns [`PositionError`] for a position that is not well formed.
pub fn local_plane(
    latitude_deg: f64,
    longitude_deg: f64,
    height_msl_m: f64,
) -> Result<LocalTangentPlane, PositionError> {
    LocalTangentPlane::new(world_position(latitude_deg, longitude_deg, height_msl_m))
}

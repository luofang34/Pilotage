//! The body that Pilotage flies on, for each Navigate call that names one.
//!
//! Navigate requires an explicit ellipsoid and surface gravity (Navigate
//! ADR-0013 and ADR-0015). Pilotage flies on the Earth, so every such
//! argument in this crate comes from this module, and no call site
//! selects a body of its own.

use navigate_contract::{Ellipsoid, GeodeticPosition};
use navigate_geodesy::{BodyEllipsoid, GeodesyError, LocalTangentPlane};

/// The reference ellipsoid of every plan and published solution.
pub(crate) const ELLIPSOID: Ellipsoid = Ellipsoid::WGS84;

/// The same ellipsoid with the derived constants that geodesy uses.
pub(crate) const GEODESY: BodyEllipsoid = BodyEllipsoid::WGS84;

/// The surface gravity that sizes a fly-by turn, in m/s².
pub(crate) const SURFACE_GRAVITY_MPS2: f64 = navigate_route::EARTH_STANDARD_GRAVITY_MPS2;

/// A local tangent plane at `origin` on [`GEODESY`].
pub(crate) fn tangent_plane(origin: GeodeticPosition) -> Result<LocalTangentPlane, GeodesyError> {
    LocalTangentPlane::new(origin, GEODESY)
}

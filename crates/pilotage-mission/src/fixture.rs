//! Deterministic demo navdata: three waypoints and one airway laid out
//! around a caller-chosen anchor, packed through the same blob container
//! as published data.
//!
//! Geometry uses [`navigate_geodesy::LocalTangentPlane`] at the anchor —
//! exact NED-to-geodetic, no small-angle shortcuts — so the offsets
//! below are true meters at any plausible anchor.

use aerocontext_core::{
    Airway, AirwayLocation, AirwayPoint, GeoPoint, NavDataCycle, NavDataSnapshot, NavPoint,
    NavPointKind,
};
use aerocontext_navdata::blob;
use chrono::NaiveDate;
use navigate_contract::GeodeticPosition;
use navigate_geodesy::{LocalTangentPlane, NedOffset};

use crate::error::MissionBuildError;

/// An anchor in the degree convention navdata snapshots use. The demo
/// snapshot is the one place degrees enter; plan build converts to
/// radians once (ADR-0030).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPointDegrees {
    /// Latitude in degrees, north positive.
    pub lat_deg: f64,
    /// Longitude in degrees, east positive.
    pub lon_deg: f64,
    /// Height above the WGS84 ellipsoid, meters.
    pub alt_m: f64,
}

/// The demo airway designator. The route tokenizer only recognizes
/// published family prefixes (V/J/T/Q and the letter families) plus
/// digits, so the demo airway must wear a real family shape; `V901`
/// exists only inside the fixture snapshot.
pub const DEMO_AIRWAY: &str = "V901";

/// The demo route: entry fix, airway traversal, exit fix — exercising
/// airway expansion, which must emit `DEMOB` between the two.
pub const DEMO_ROUTE: &str = "DEMOA V901 DEMOC";

/// The demo cycle's fixed effective date: 2026-01-22, an FAA NASR
/// effective date on the 28-day AIRAC grid.
pub const DEMO_EFFECTIVE_DATE: NaiveDate = match NaiveDate::from_ymd_opt(2026, 1, 22) {
    Some(date) => date,
    None => NaiveDate::MIN,
};

/// NED offsets of the demo waypoints from the anchor, meters:
/// `DEMOA` ~120 m northeast, `DEMOB` 250 m east, `DEMOC` ~180 m
/// southeast.
const DEMO_OFFSETS: [(&str, f64, f64); 3] = [
    ("DEMOA", 85.0, 85.0),
    ("DEMOB", 0.0, 250.0),
    ("DEMOC", -127.0, 127.0),
];

/// Builds the demo snapshot: `DEMOA`/`DEMOB`/`DEMOC` around `anchor`
/// plus the airway [`DEMO_AIRWAY`] joining them in order.
///
/// # Errors
///
/// [`MissionBuildError::Geodesy`] for an implausible anchor;
/// [`MissionBuildError::Cycle`] cannot occur for the fixed demo date but
/// is propagated rather than swallowed.
pub fn demo_snapshot(anchor_deg: GeoPointDegrees) -> Result<NavDataSnapshot, MissionBuildError> {
    let origin = GeodeticPosition::new(
        anchor_deg.lat_deg.to_radians(),
        anchor_deg.lon_deg.to_radians(),
        anchor_deg.alt_m,
    );
    let plane = LocalTangentPlane::new(origin)?;
    let points = DEMO_OFFSETS
        .iter()
        .map(|&(ident, north_m, east_m)| {
            let position = plane.from_ned(&NedOffset::new(north_m, east_m, 0.0));
            NavPoint::new(
                ident,
                NavPointKind::Waypoint,
                GeoPoint {
                    lat: position.latitude_rad.to_degrees(),
                    lon: position.longitude_rad.to_degrees(),
                },
            )
        })
        .collect();
    let airway = Airway::new(
        DEMO_AIRWAY,
        AirwayLocation::Conus,
        DEMO_OFFSETS
            .iter()
            .map(|&(ident, _, _)| AirwayPoint::new(ident))
            .collect(),
    );
    let cycle = NavDataCycle::faa_nasr(DEMO_EFFECTIVE_DATE)?;
    Ok(NavDataSnapshot::new(cycle, points).with_airways(vec![airway]))
}

/// [`demo_snapshot`] packed through the versioned blob container, so the
/// demo path exercises exactly the decode-and-verify road published data
/// travels.
///
/// # Errors
///
/// [`demo_snapshot`]'s errors, plus [`MissionBuildError::Blob`] should
/// the container refuse to encode.
pub fn demo_blob(anchor_deg: GeoPointDegrees) -> Result<Vec<u8>, MissionBuildError> {
    let snapshot = demo_snapshot(anchor_deg)?;
    Ok(blob::encode(&snapshot)?)
}

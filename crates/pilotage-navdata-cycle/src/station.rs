use aerocontext_core::{NavDataSnapshot, NavPointKind};

/// Where one reporting station sits.
#[derive(Clone, Debug, PartialEq)]
pub struct StationPosition {
    /// Published station identifier, as a weather report names it.
    pub station_id: String,
    /// WGS84 latitude in degrees.
    pub latitude_deg: f64,
    /// WGS84 longitude in degrees.
    pub longitude_deg: f64,
}

/// Positions for the stations a weather report can name.
///
/// A text weather report names its station and carries no position, so a client with no
/// navigation data draws no weather however well the report decoded. An airport is the
/// facility that files a METAR, a TAF, and a SPECI, so an airport point is the position a
/// station identifier resolves to.
///
/// Waypoints and navaids are excluded. They share the identifier space and never file a
/// report, so including them would place a report at a fix that happens to share its name.
#[must_use]
pub fn weather_station_positions(snapshot: &NavDataSnapshot) -> Vec<StationPosition> {
    snapshot
        .points
        .iter()
        .filter(|point| point.kind == NavPointKind::Airport)
        .map(|point| StationPosition {
            station_id: point.ident.clone(),
            latitude_deg: point.position.lat,
            longitude_deg: point.position.lon,
        })
        .collect()
}

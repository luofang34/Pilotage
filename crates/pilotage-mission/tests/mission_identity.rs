//! A mission's identity is its content digest. Stored records and mission
//! documents carry it, so a change in how a plan is held must not change
//! it.
//!
//! The fixture gives every waypoint in degrees and converts nothing with a
//! transcendental function. The last bits of such a function differ
//! between platform math libraries, and a digest of those bits would
//! differ between hosts for one tree.

#![cfg(test)]

use aerocontext_core::{
    Airway, AirwayLocation, AirwayPoint, GeoPoint, NavDataCycle, NavDataSnapshot, NavPoint,
    NavPointKind,
};
use aerocontext_navdata::blob;
use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_mission::fixture::{DEMO_AIRWAY, DEMO_EFFECTIVE_DATE, DEMO_ROUTE};
use pilotage_mission::{MissionConfig, MissionEngine, decode_snapshot};

/// The waypoints of the demo route, in degrees.
const POINTS: [(&str, f64, f64); 3] = [
    ("DEMOA", 47.00075, 8.0011),
    ("DEMOB", 47.0, 8.0033),
    ("DEMOC", 46.99886, 8.00167),
];

/// The content digest of the demo route mission over [`POINTS`].
const ROUTE_DIGEST: &str = "9259409faa70436761c24a2a2da46953cb0a7af7573a4071c1e0f8f015c6f03f";

fn snapshot_blob() -> Vec<u8> {
    let points = POINTS
        .iter()
        .map(|&(ident, lat, lon)| {
            NavPoint::new(ident, NavPointKind::Waypoint, GeoPoint { lat, lon })
        })
        .collect();
    let airway = Airway::new(
        DEMO_AIRWAY,
        AirwayLocation::Conus,
        POINTS
            .iter()
            .map(|&(ident, _, _)| AirwayPoint::new(ident))
            .collect(),
    );
    let cycle = NavDataCycle::faa_nasr(DEMO_EFFECTIVE_DATE).expect("the demo cycle is valid");
    let snapshot = NavDataSnapshot::new(cycle, points).with_airways(vec![airway]);
    blob::encode(&snapshot).expect("the snapshot encodes")
}

#[test]
fn the_route_mission_keeps_its_content_digest() {
    let (snapshot, provenance) =
        decode_snapshot(&snapshot_blob(), true).expect("the snapshot decodes");
    let anchor = GeodeticPosition::new(47.0_f64.to_radians(), 8.0_f64.to_radians(), 500.0);
    let config = MissionConfig::new(DEMO_ROUTE.to_owned(), anchor, ClockDomainId::new(7));
    let (_, record) = MissionEngine::new(&snapshot, provenance, config).expect("mission builds");
    assert_eq!(
        record.mission_identity.content_digest.to_string(),
        ROUTE_DIGEST
    );
}

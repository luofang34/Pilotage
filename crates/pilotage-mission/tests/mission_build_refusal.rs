//! Mission-build refusals: a route string that cannot become a mission
//! must fail typed, naming what is wrong, rather than build a mission
//! that silently omits part of the route.

#![allow(clippy::expect_used, clippy::panic)]

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{MissionBuildError, MissionConfig, MissionEngine, decode_snapshot};

/// The anchor every scenario flies from, matching the closed-loop tests
/// in `mission_engine.rs` so both suites read the same geometry.
const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};

#[test]
fn mission_build_refuses_a_route_with_an_unexecuted_procedure() {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let anchor = GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    );
    // BLUES2.IIU is dot-notation, recognized as a procedure token
    // wherever it appears. TRUKN2 is not in the demo snapshot; as an
    // edge token ending in a digit, the expander reclassifies it as a
    // SID/STAR computer code rather than failing the expansion.
    let route = format!("{} BLUES2.IIU TRUKN2", fixture::DEMO_ROUTE);
    let config = MissionConfig::new(route.clone(), anchor, ClockDomainId::new(7));

    let error = MissionEngine::new(&snapshot, provenance, config)
        .expect_err("a route carrying an unexecuted procedure must be refused");

    match error {
        MissionBuildError::UnexecutedProcedure {
            route: got_route,
            procedures,
        } => {
            assert_eq!(got_route, route);
            assert_eq!(
                procedures,
                vec!["BLUES2.IIU".to_owned(), "TRUKN2".to_owned()]
            );
        }
        other => panic!("expected UnexecutedProcedure, got {other:?}"),
    }
}

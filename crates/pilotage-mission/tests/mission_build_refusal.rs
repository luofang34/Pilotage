//! Mission-build refusals: a route string that cannot become a mission
//! must fail typed, naming what is wrong, rather than build a mission
//! that silently omits part of the route.

#![allow(clippy::expect_used, clippy::panic)]

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{
    ExecutionTarget, MissionBuildError, MissionConfig, MissionEngine, OwnshipSource,
    PlannedWaypoint, SurveyRequest, WaypointPlan, decode_snapshot,
};

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

/// A config for `target` that flies on a simulator's truth oracle.
fn truth_config(target: ExecutionTarget) -> MissionConfig {
    let anchor = GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    );
    let mut config = MissionConfig::new(
        fixture::DEMO_ROUTE.to_owned(),
        anchor,
        ClockDomainId::new(7),
    );
    config.ownship_source = OwnshipSource::SimulationTruth;
    config.execution_target = target;
    config
}

fn survey_request() -> SurveyRequest {
    let side = 0.001;
    SurveyRequest::new(vec![
        [ANCHOR.lat_deg, ANCHOR.lon_deg],
        [ANCHOR.lat_deg + side, ANCHOR.lon_deg],
        [ANCHOR.lat_deg + side, ANCHOR.lon_deg + side],
        [ANCHOR.lat_deg, ANCHOR.lon_deg + side],
    ])
}

fn waypoint_plan() -> WaypointPlan {
    WaypointPlan::new(vec![
        PlannedWaypoint::new(ANCHOR.lat_deg + 0.001, ANCHOR.lon_deg, ANCHOR.alt_m + 30.0),
        PlannedWaypoint::new(ANCHOR.lat_deg + 0.002, ANCHOR.lon_deg, ANCHOR.alt_m + 30.0),
    ])
}

#[test]
fn every_builder_refuses_simulation_truth_for_a_real_vehicle() {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let config = || truth_config(ExecutionTarget::RealVehicle);
    let refusals = [
        MissionEngine::new(&snapshot, provenance, config()).map(drop),
        MissionEngine::from_survey(&survey_request(), config()).map(drop),
        MissionEngine::from_waypoints(&waypoint_plan(), config()).map(drop),
        MissionEngine::home(config()).map(drop),
    ];
    for refusal in refusals {
        assert!(
            matches!(refusal, Err(MissionBuildError::TruthOwnshipOnRealVehicle)),
            "expected the truth-source refusal, got {refusal:?}"
        );
    }
}

#[test]
fn every_builder_accepts_simulation_truth_for_a_simulator() {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let config = || truth_config(ExecutionTarget::Simulator);
    MissionEngine::new(&snapshot, provenance, config()).expect("route builds");
    MissionEngine::from_survey(&survey_request(), config()).expect("survey builds");
    MissionEngine::from_waypoints(&waypoint_plan(), config()).expect("waypoints build");
    MissionEngine::home(config()).expect("home builds");
}

#[test]
fn the_default_source_is_the_vehicle_estimate_and_a_real_vehicle_builds_on_it() {
    let mut config = truth_config(ExecutionTarget::RealVehicle);
    let default = MissionConfig::new(String::new(), config.anchor, config.clock);
    assert_eq!(default.ownship_source, OwnshipSource::vehicle_estimate());
    config.ownship_source = default.ownship_source;
    MissionEngine::home(config).expect("a real vehicle flies on its own estimate");
}

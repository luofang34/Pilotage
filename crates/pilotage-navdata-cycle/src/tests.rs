//! Cycle loading and station lookup tests.

#![allow(clippy::expect_used, clippy::panic)]

use aerocontext_core::{GeoPoint, NavDataCycle, NavDataSnapshot, NavPoint, NavPointKind};
use chrono::NaiveDate;

use crate::{CycleLoadError, load_cycle_bytes, weather_station_positions};

fn cycle() -> NavDataCycle {
    let effective_on = NaiveDate::from_ymd_opt(2026, 6, 11).expect("valid date");
    NavDataCycle::faa_nasr(effective_on).expect("valid cycle")
}

fn point(ident: &str, kind: NavPointKind, lat: f64, lon: f64) -> NavPoint {
    NavPoint::new(ident, kind, GeoPoint { lat, lon })
}

fn snapshot() -> NavDataSnapshot {
    NavDataSnapshot::new(
        cycle(),
        vec![
            point("KPHL", NavPointKind::Airport, 39.87, -75.24),
            point("KTTN", NavPointKind::Airport, 40.27, -74.81),
            point("ARD", NavPointKind::Navaid, 39.86, -74.90),
            point("DITCH", NavPointKind::Waypoint, 39.50, -74.50),
        ],
    )
}

#[test]
fn an_encoded_cycle_loads_back() {
    let bytes = aerocontext_navdata::encode(&snapshot()).expect("encodes");
    let loaded = load_cycle_bytes("memory", &bytes).expect("decodes");
    assert_eq!(loaded.points.len(), 4);
    assert_eq!(loaded.cycle.effective_on, cycle().effective_on);
}

#[test]
fn bytes_that_are_not_a_cycle_report_the_file() {
    // The client loads a cycle from an application asset, and an asset replaced by the
    // wrong file must name itself rather than fail somewhere later.
    let error = load_cycle_bytes("cycle.acnav", b"not a cycle").expect_err("rejects");
    let CycleLoadError::Decode { path, .. } = error else {
        panic!("expected a decode failure");
    };
    assert_eq!(path.to_string_lossy(), "cycle.acnav");
}

#[test]
fn only_an_airport_answers_for_a_reporting_station() {
    // A navaid and a waypoint share the identifier space with an airport and never file a
    // report. Placing a report at one of those would put weather at the wrong point.
    let positions = weather_station_positions(&snapshot());
    let idents: Vec<&str> = positions
        .iter()
        .map(|position| position.station_id.as_str())
        .collect();
    assert_eq!(idents, vec!["KPHL", "KTTN"]);
    assert!((positions[0].latitude_deg - 39.87).abs() < 1e-9);
    assert!((positions[0].longitude_deg + 75.24).abs() < 1e-9);
}

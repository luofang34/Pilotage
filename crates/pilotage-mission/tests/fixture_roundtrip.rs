//! Fixture blob round-trip and airway expansion through the public API.

#![allow(clippy::expect_used, clippy::panic)]

use aerocontext_planning::route::expand_str;
use chrono::NaiveDate;
use pilotage_mission::decode_snapshot;
use pilotage_mission::fixture::{self, GeoPointDegrees};

const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};

#[test]
fn demo_blob_round_trips_with_faithful_provenance() {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (decoded, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let original = fixture::demo_snapshot(ANCHOR).expect("demo snapshot builds");
    assert_eq!(decoded, original, "container round-trip is lossless");
    assert!(provenance.fixture);
    assert_eq!(provenance.authority, "faa-nasr");
    assert_eq!(provenance.effective_on, fixture::DEMO_EFFECTIVE_DATE);
    assert_eq!(
        provenance.next_effective_on,
        NaiveDate::from_ymd_opt(2026, 2, 19).expect("valid date"),
        "28-day cycle window"
    );
    assert_eq!(provenance.sha256_hex.len(), 64);
    assert!(provenance.sha256_hex.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(
        provenance.navigation_data_identity.cycle,
        "faa-nasr:2026-01-22"
    );
    assert!(
        provenance
            .navigation_data_identity
            .snapshot_id
            .ends_with(&provenance.sha256_hex)
    );
    assert_eq!(
        provenance
            .navigation_data_identity
            .snapshot_digest
            .to_string(),
        provenance.sha256_hex
    );
}

#[test]
fn demo_route_expands_through_the_airway_in_order() {
    let snapshot = fixture::demo_snapshot(ANCHOR).expect("demo snapshot builds");
    let expanded = expand_str(fixture::DEMO_ROUTE, &snapshot).expect("route expands");
    let idents: Vec<Option<&str>> = expanded
        .points
        .iter()
        .map(|point| point.ident.as_deref())
        .collect();
    assert_eq!(
        idents,
        vec![Some("DEMOA"), Some("DEMOB"), Some("DEMOC")],
        "airway traversal emits the intermediate fix"
    );
    let via: Vec<Option<&str>> = expanded
        .points
        .iter()
        .map(|point| point.via_airway.as_deref())
        .collect();
    assert_eq!(
        via,
        vec![None, Some(fixture::DEMO_AIRWAY), Some(fixture::DEMO_AIRWAY)],
        "DEMOB and DEMOC come from the airway, DEMOA from its own token"
    );
    assert!(expanded.procedures.is_empty());
}

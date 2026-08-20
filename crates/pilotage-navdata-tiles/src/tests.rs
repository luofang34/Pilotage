//! Tests for deterministic offline Navdata tile bundles.

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use aerocontext_core::navdata::{
    Airspace, AirspaceKind, Airway, AirwayLocation, AirwayPoint, ControlledClass, NavPoint,
    NavPointKind, Runway,
};
use aerocontext_core::{Area, GeoPoint, NavDataCycle, NavDataSnapshot};
use chrono::NaiveDate;
use flate2::read::GzDecoder;
use pilotage_airspace_view::{IdentifiedNavdataSnapshotV1, NavdataIdentityV1, navdata_cycle_id};
use prost::Message;
use rusqlite::{Connection, MAIN_DB};

use super::*;
use crate::mvt::proto;

fn test_config() -> NavdataTileConfig {
    NavdataTileConfig {
        airspace_min_zoom: 0,
        airway_min_zoom: 0,
        aerodrome_min_zoom: 0,
        navaid_min_zoom: 0,
        fix_min_zoom: 0,
        max_zoom: 0,
    }
}

fn identified() -> IdentifiedNavdataSnapshotV1 {
    let cycle =
        NavDataCycle::faa_nasr(NaiveDate::from_ymd_opt(2026, 6, 11).expect("valid test date"))
            .expect("valid cycle");
    let points = vec![
        point("KBOS", NavPointKind::Airport, 42.3656, -71.0096, "K1"),
        point("PUT", NavPointKind::Navaid, 41.9555, -71.8443, "K1"),
        point("DREEM", NavPointKind::Waypoint, 42.2, -71.5, "K1"),
        point(
            "PRIVATE",
            NavPointKind::Other("private-use".to_owned()),
            42.0,
            -71.0,
            "K1",
        ),
    ];
    let airway = Airway::new(
        "V1",
        AirwayLocation::Conus,
        vec![
            AirwayPoint::new("PUT").with_icao_region(Some("K1".to_owned())),
            AirwayPoint::new("DREEM").with_icao_region(Some("K1".to_owned())),
        ],
    );
    let airspace = Airspace::new(AirspaceKind::Controlled(ControlledClass::B), "BOS-B")
        .with_center_ident(Some("KBOS".to_owned()))
        .with_bounds(Some(Area::BoundingBox {
            south_west: GeoPoint {
                lat: 42.0,
                lon: -71.5,
            },
            north_east: GeoPoint {
                lat: 42.7,
                lon: -70.5,
            },
        }));
    let snapshot = NavDataSnapshot::new(cycle, points)
        .with_airways(vec![airway])
        .with_runways(vec![Runway::new("KBOS", "04L/22R")])
        .with_airspaces(vec![airspace]);
    let identity = NavdataIdentityV1 {
        cycle: navdata_cycle_id(&snapshot),
        snapshot_id: "snapshot-test-1".to_owned(),
        snapshot_digest: "sha256:test-digest".to_owned(),
    };
    IdentifiedNavdataSnapshotV1::try_new(identity, snapshot).expect("matching cycle")
}

fn point(ident: &str, kind: NavPointKind, lat: f64, lon: f64, region: &str) -> NavPoint {
    NavPoint::new(ident, kind, GeoPoint { lat, lon }).with_region(Some(region.to_owned()))
}

#[test]
fn same_snapshot_produces_identical_archive_bytes() {
    let snapshot = identified();
    let first = build_mbtiles(&snapshot, test_config()).expect("first build");
    let second = build_mbtiles(&snapshot, test_config()).expect("second build");

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(&first.bytes()[..16], b"SQLite format 3\0");
    assert_eq!(first.report().tile_count, 1);
    assert_eq!(first.report().archive_bytes, first.bytes().len() as u64);
}

#[test]
fn archive_carries_identity_and_reads_without_a_network() {
    let snapshot = identified();
    let bundle = build_mbtiles(&snapshot, test_config()).expect("tile build");
    let reader = OfflineTileReader::from_bytes(bundle.bytes()).expect("offline reader");

    assert_eq!(reader.identity(), snapshot.identity());
    let tile = reader
        .tile_xyz(0, 0, 0)
        .expect("tile query")
        .expect("global tile");
    assert_eq!(&tile[..3], &[0x1f, 0x8b, 0x08]);
    assert!(reader.tile_xyz(1, 2, 0).is_err());
}

#[test]
fn every_drawable_feature_has_cycle_scoped_stable_subject_properties() {
    let snapshot = identified();
    let bundle = build_mbtiles(&snapshot, test_config()).expect("tile build");
    let reader = OfflineTileReader::from_bytes(bundle.bytes()).expect("offline reader");
    let compressed = reader
        .tile_xyz(0, 0, 0)
        .expect("tile query")
        .expect("global tile");
    let tile = decode_tile(&compressed);
    let mut seen_layers = Vec::new();

    for layer in &tile.layers {
        seen_layers.push(layer.name.as_str());
        for feature in &layer.features {
            let properties = properties(layer, feature);
            assert!(properties["subject_id"].starts_with("subject-v1|"));
            assert_eq!(properties["subject_cycle"], snapshot.identity().cycle);
            if layer.name == "airways" {
                assert_eq!(properties["subject_id"], "subject-v1|airway|1:C|2:V1|0:");
            }
            assert!(feature.id.is_some());
        }
    }
    seen_layers.sort_unstable();
    assert_eq!(
        seen_layers,
        ["aerodromes", "airspaces", "airways", "fixes", "navaids"]
    );
}

#[test]
fn report_states_drawable_and_typed_omitted_records() {
    let bundle = build_mbtiles(&identified(), test_config()).expect("tile build");
    let report = bundle.report();

    assert_eq!(report.features.total(), 5);
    assert_eq!(report.features.aerodromes, 1);
    assert_eq!(report.features.navaids, 1);
    assert_eq!(report.features.fixes, 1);
    assert_eq!(report.features.airways, 1);
    assert_eq!(report.features.airspaces, 1);
    assert_eq!(report.omitted.other_points, 1);
    assert_eq!(report.omitted.runways_without_geometry, 1);
    assert_eq!(report.omitted.unresolved_airways, 0);
}

#[test]
fn published_airway_gap_does_not_create_a_segment() {
    let source = identified();
    let mut raw = source.snapshot().clone();
    raw.airways[0].points[0].gap_to_next = true;
    let snapshot = IdentifiedNavdataSnapshotV1::try_new(source.identity().clone(), raw)
        .expect("matching cycle");
    let bundle = build_mbtiles(&snapshot, test_config()).expect("tile build");

    assert_eq!(bundle.report().features.airways, 0);
    assert_eq!(bundle.report().omitted.unresolved_airways, 1);
}

#[test]
fn empty_snapshot_identity_is_refused() {
    let source = identified();
    let identity = NavdataIdentityV1 {
        cycle: source.identity().cycle.clone(),
        snapshot_id: String::new(),
        snapshot_digest: "digest".to_owned(),
    };
    let snapshot = IdentifiedNavdataSnapshotV1::try_new(identity, source.snapshot().clone())
        .expect("cycle still matches");
    let error = build_mbtiles(&snapshot, test_config()).expect_err("empty identity must fail");

    assert!(matches!(
        error,
        NavdataTileError::EmptyIdentity {
            field: "snapshot_id"
        }
    ));
}

#[test]
fn gzip_header_is_reproducible() {
    let bundle = build_mbtiles(&identified(), test_config()).expect("tile build");
    let reader = OfflineTileReader::from_bytes(bundle.bytes()).expect("offline reader");
    let tile = reader
        .tile_xyz(0, 0, 0)
        .expect("tile query")
        .expect("global tile");

    assert_eq!(&tile[4..8], &[0, 0, 0, 0]);
    assert_eq!(tile[9], 255);
}

#[test]
fn invalid_coordinate_is_a_typed_build_failure() {
    let source = identified();
    let mut raw = source.snapshot().clone();
    raw.points[0].position.lat = 91.0;
    let snapshot = IdentifiedNavdataSnapshotV1::try_new(source.identity().clone(), raw)
        .expect("matching cycle");
    let error = build_mbtiles(&snapshot, test_config()).expect_err("latitude must fail");

    assert!(matches!(
        error,
        NavdataTileError::InvalidCoordinate { latitude: 91.0, .. }
    ));
}

#[test]
fn invalid_zoom_policy_is_refused_before_tiling() {
    let mut config = test_config();
    config.max_zoom = 15;
    let error = build_mbtiles(&identified(), config).expect_err("zoom must fail");

    assert!(matches!(error, NavdataTileError::InvalidConfig { .. }));
}

#[test]
fn connected_airway_segments_use_one_mvt_path() {
    let source = identified();
    let mut raw = source.snapshot().clone();
    raw.points
        .push(point("JFK", NavPointKind::Navaid, 40.6398, -73.7789, "K1"));
    raw.airways[0]
        .points
        .push(AirwayPoint::new("JFK").with_icao_region(Some("K1".to_owned())));
    let snapshot = IdentifiedNavdataSnapshotV1::try_new(source.identity().clone(), raw)
        .expect("matching cycle");
    let bundle = build_mbtiles(&snapshot, test_config()).expect("tile build");
    let reader = OfflineTileReader::from_bytes(bundle.bytes()).expect("offline reader");
    let compressed = reader
        .tile_xyz(0, 0, 0)
        .expect("tile query")
        .expect("global tile");
    let tile = decode_tile(&compressed);
    let airway = tile
        .layers
        .iter()
        .find(|layer| layer.name == "airways")
        .and_then(|layer| layer.features.first())
        .expect("airway feature");

    assert_eq!(airway.geometry[0], 9);
    assert_eq!(airway.geometry[3], 18);
}

#[test]
fn vector_manifest_lists_only_layers_that_occur() {
    let source = identified();
    let mut raw = source.snapshot().clone();
    raw.airspaces.clear();
    let snapshot = IdentifiedNavdataSnapshotV1::try_new(source.identity().clone(), raw)
        .expect("matching cycle");
    let bundle = build_mbtiles(&snapshot, test_config()).expect("tile build");
    let mut connection = Connection::open_in_memory().expect("in-memory database");
    connection
        .deserialize_read_exact(
            MAIN_DB,
            Cursor::new(bundle.bytes()),
            bundle.bytes().len(),
            true,
        )
        .expect("MBTiles bytes");
    let json: String = connection
        .query_row(
            "SELECT value FROM metadata WHERE name = 'json'",
            [],
            |row| row.get(0),
        )
        .expect("vector metadata");

    assert!(!json.contains("airspaces"));
    assert!(json.contains("airways"));
}

fn decode_tile(compressed: &[u8]) -> proto::Tile {
    let mut decoder = GzDecoder::new(compressed);
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).expect("gzip tile");
    proto::Tile::decode(bytes.as_slice()).expect("MVT tile")
}

fn properties(layer: &proto::Layer, feature: &proto::Feature) -> BTreeMap<String, String> {
    feature
        .tags
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let key = layer.keys[pair[0] as usize].clone();
            let value = layer.values[pair[1] as usize]
                .string_value
                .clone()
                .expect("string property");
            (key, value)
        })
        .collect()
}

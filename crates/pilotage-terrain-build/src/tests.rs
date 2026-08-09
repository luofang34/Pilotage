//! Tests for deterministic Terrarium archive construction.

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;

use pilotage_geo::{DatumRealizationId, GeoidModelId, HorizontalDatum, VerticalDatum};
use pilotage_svs_build::{Accuracy, LicenseCode, SourceId, SourceMeta, TerrainGrid};
use rusqlite::Connection;
use tempfile::NamedTempFile;

use super::*;
use crate::tile::terrarium_rgb;

const SOURCE_ID: SourceId = SourceId(41);

fn config() -> TerrainBuildConfig {
    TerrainBuildConfig {
        region: TerrainRegion {
            min_lat_deg: -WEB_MERCATOR_MAX_LAT_DEG,
            max_lat_deg: WEB_MERCATOR_MAX_LAT_DEG,
            min_lon_deg: -180.0,
            max_lon_deg: 180.0,
        },
        min_zoom: 0,
        max_zoom: 0,
    }
}

fn dataset() -> SourceDataset {
    let rows = 3u32;
    let cols = 5u32;
    let mut posts = Vec::with_capacity((rows * cols) as usize);
    for row in 0..rows {
        for col in 0..cols {
            posts.push(Some(100.0 + f64::from(row * 20 + col * 3)));
        }
    }
    SourceDataset {
        meta: vec![SourceMeta {
            id: SOURCE_ID,
            version: 1,
            license: LicenseCode::Open,
            horizontal_datum: HorizontalDatum::Wgs84,
            realization: DatumRealizationId::UNDECLARED,
            vertical_datum: VerticalDatum::Msl,
            geoid: GeoidModelId(7),
            accuracy: Accuracy {
                horizontal_mm: 30_000,
                vertical_mm: 10_000,
            },
        }],
        terrain: vec![TerrainGrid {
            source: SOURCE_ID,
            origin_lat_deg: -90.0,
            origin_lon_deg: -180.0,
            step_deg: 90.0,
            rows,
            cols,
            posts,
        }],
        obstacles: Vec::new(),
        aerodromes: Vec::new(),
    }
}

#[test]
fn same_source_and_config_produce_identical_archive_bytes() {
    let source = dataset();
    let first = build_mbtiles(&source, config()).expect("first terrain build");
    let second = build_mbtiles(&source, config()).expect("second terrain build");

    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.report().tile_count, 1);
    assert_eq!(first.report().archive_bytes, first.bytes().len() as u64);
    assert_eq!(&first.bytes()[..16], b"SQLite format 3\0");
}

#[test]
fn archive_has_terrarium_metadata_and_one_png_tile() {
    let bundle = build_mbtiles(&dataset(), config()).expect("terrain build");
    let file = NamedTempFile::new().expect("temporary archive");
    fs::write(file.path(), bundle.bytes()).expect("write archive");
    let connection = Connection::open(file.path()).expect("open archive");

    let encoding: String = connection
        .query_row(
            "SELECT value FROM metadata WHERE name = 'encoding'",
            [],
            |row| row.get(0),
        )
        .expect("encoding metadata");
    let tile: Vec<u8> = connection
        .query_row(
            "SELECT tile_data FROM tiles
             WHERE zoom_level = 0 AND tile_column = 0 AND tile_row = 0",
            [],
            |row| row.get(0),
        )
        .expect("tile bytes");

    assert_eq!(encoding, "terrarium");
    assert_eq!(&tile[..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn mbtiles_rows_use_the_tms_origin() {
    let north_west = TerrainBuildConfig {
        region: TerrainRegion {
            min_lat_deg: 0.0,
            max_lat_deg: WEB_MERCATOR_MAX_LAT_DEG,
            min_lon_deg: -180.0,
            max_lon_deg: 0.0,
        },
        min_zoom: 1,
        max_zoom: 1,
    };
    let bundle = build_mbtiles(&dataset(), north_west).expect("north-west terrain build");
    let file = NamedTempFile::new().expect("temporary archive");
    fs::write(file.path(), bundle.bytes()).expect("write archive");
    let connection = Connection::open(file.path()).expect("open archive");
    let count: u32 = connection
        .query_row(
            "SELECT COUNT(*) FROM tiles
             WHERE zoom_level = 1 AND tile_column = 0 AND tile_row = 1",
            [],
            |row| row.get(0),
        )
        .expect("TMS tile row");

    assert_eq!(count, 1);
}

#[test]
fn selected_tiles_require_complete_source_coverage() {
    let mut source = dataset();
    source.terrain[0].posts[0] = None;

    let error = build_mbtiles(&source, config()).expect_err("void must fail the build");
    assert!(matches!(error, TerrainBuildError::MissingElevation { .. }));
}

#[test]
fn terrarium_encoding_rounds_to_one_over_256_meter() {
    let elevation_m = 1_234.567;
    let [red, green, blue] = terrarium_rgb(elevation_m).expect("Terrarium code");
    let decoded = f64::from(red) * 256.0 + f64::from(green) + f64::from(blue) / 256.0 - 32_768.0;

    assert!((decoded - elevation_m).abs() <= 1.0 / 512.0);
}

#[test]
fn invalid_region_is_refused_before_rasterization() {
    let mut bad = config();
    bad.region.max_lat_deg = 90.0;

    let error = build_mbtiles(&dataset(), bad).expect_err("invalid region must fail");
    assert!(matches!(error, TerrainBuildError::InvalidConfig { .. }));
}

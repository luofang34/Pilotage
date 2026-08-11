//! Tests for runtime Terrarium archive queries.

#![allow(clippy::expect_used, clippy::panic)]

use png::{BitDepth, ColorType, Encoder};
use rusqlite::{Connection, params};
use tempfile::NamedTempFile;

use crate::{TerrainArchive, TerrainQueryError};

#[test]
fn deepest_covering_tile_supplies_elevation() {
    let file = archive(&[(0, 0, 0, 100.0), (1, 1, 0, 400.0)]);
    let mut terrain = TerrainArchive::open_blocking(file.path()).expect("open terrain archive");

    let elevation = terrain
        .elevation_m_blocking(-0.1, 0.1)
        .expect("query terrain")
        .expect("terrain coverage");

    assert!((elevation - 400.0).abs() <= 1.0 / 256.0);
}

#[test]
fn missing_deep_tile_uses_a_less_detailed_tile() {
    let file = archive(&[(0, 0, 0, 1_000.0), (1, 0, 1, 200.0)]);
    let mut terrain = TerrainArchive::open_blocking(file.path()).expect("open terrain archive");

    let elevation = terrain
        .elevation_m_blocking(-0.1, 0.1)
        .expect("query terrain")
        .expect("world tile coverage");

    assert!((elevation - 1_000.0).abs() <= 1.0 / 256.0);
}

#[test]
fn no_covering_tile_is_an_explicit_absence() {
    let file = archive(&[(1, 0, 1, 200.0)]);
    let mut terrain = TerrainArchive::open_blocking(file.path()).expect("open terrain archive");

    let elevation = terrain
        .elevation_m_blocking(-0.1, 0.1)
        .expect("query terrain");

    assert_eq!(elevation, None);
}

#[test]
fn a_non_terrarium_archive_is_rejected() {
    let file = archive(&[(0, 0, 0, 0.0)]);
    let connection = Connection::open(file.path()).expect("open fixture archive");
    connection
        .execute(
            "UPDATE metadata SET value = 'mapbox' WHERE name = 'encoding'",
            [],
        )
        .expect("change fixture metadata");
    drop(connection);

    let error = TerrainArchive::open_blocking(file.path())
        .err()
        .expect("encoding must be checked");

    assert!(matches!(
        error,
        TerrainQueryError::UnsupportedMetadata {
            name: "encoding",
            ..
        }
    ));
}

fn archive(tiles: &[(u8, u32, u32, f64)]) -> NamedTempFile {
    let file = NamedTempFile::new().expect("create fixture archive");
    let connection = Connection::open(file.path()).expect("open fixture archive");
    connection
        .execute_batch(
            "CREATE TABLE metadata (name TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE tiles (
                 zoom_level INTEGER NOT NULL,
                 tile_column INTEGER NOT NULL,
                 tile_row INTEGER NOT NULL,
                 tile_data BLOB NOT NULL,
                 PRIMARY KEY (zoom_level, tile_column, tile_row)
             );",
        )
        .expect("create fixture schema");
    let max_zoom = tiles.iter().map(|tile| tile.0).max().unwrap_or(0);
    for (name, value) in [
        ("encoding", "terrarium".to_owned()),
        ("format", "png".to_owned()),
        ("minzoom", "0".to_owned()),
        ("maxzoom", max_zoom.to_string()),
        ("tile_size", "2".to_owned()),
    ] {
        connection
            .execute(
                "INSERT INTO metadata (name, value) VALUES (?1, ?2)",
                params![name, value],
            )
            .expect("insert fixture metadata");
    }
    for (zoom, x, tms_y, elevation_m) in tiles {
        connection
            .execute(
                "INSERT INTO tiles (zoom_level, tile_column, tile_row, tile_data)
                 VALUES (?1, ?2, ?3, ?4)",
                params![zoom, x, tms_y, terrarium_png(*elevation_m)],
            )
            .expect("insert fixture tile");
    }
    drop(connection);
    file
}

fn terrarium_png(elevation_m: f64) -> Vec<u8> {
    let code = ((elevation_m + 32_768.0) * 256.0).round() as u32;
    let pixel = [
        ((code >> 16) & 0xff) as u8,
        ((code >> 8) & 0xff) as u8,
        (code & 0xff) as u8,
    ];
    let pixels = pixel.repeat(4);
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes, 2, 2);
    encoder.set_color(ColorType::Rgb);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write fixture PNG header");
    writer
        .write_image_data(&pixels)
        .expect("write fixture PNG pixels");
    writer.finish().expect("finish fixture PNG");
    bytes
}

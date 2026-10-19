//! A Terrarium MBTiles archive written for a test.

#![allow(clippy::expect_used)]

use png::{BitDepth, ColorType, Encoder};
use rusqlite::{Connection, params};
use tempfile::NamedTempFile;

/// One fixture tile: zoom, column, and XYZ row, all at one elevation.
pub(crate) struct Tile {
    pub(crate) zoom: u8,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) elevation_m: f64,
}

impl Tile {
    /// The tile at `zoom` that covers a WGS84 position, at one elevation.
    pub(crate) fn covering(
        zoom: u8,
        latitude_deg: f64,
        longitude_deg: f64,
        elevation_m: f64,
    ) -> Self {
        let side = f64::from(1u32 << zoom);
        let x = ((longitude_deg + 180.0) / 360.0 * side).floor() as u32;
        let lat = latitude_deg.to_radians();
        let y = ((1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * side).floor() as u32;
        Self {
            zoom,
            x,
            y,
            elevation_m,
        }
    }
}

/// The ground width of a fixture tile's pixel: tiles are 2 pixels wide.
pub(crate) const TILE_SIZE: u32 = 2;

/// An archive named `name` holding `tiles`.
pub(crate) fn archive(name: &str, tiles: &[Tile]) -> NamedTempFile {
    let file = NamedTempFile::new().expect("create fixture archive");
    let connection = Connection::open(file.path()).expect("open fixture archive");
    connection
        .execute_batch(
            "CREATE TABLE metadata (name TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE tiles (zoom_level INTEGER NOT NULL, tile_column INTEGER NOT NULL,
                 tile_row INTEGER NOT NULL, tile_data BLOB NOT NULL,
                 PRIMARY KEY (zoom_level, tile_column, tile_row));",
        )
        .expect("create fixture schema");
    let max_zoom = tiles.iter().map(|tile| tile.zoom).max().unwrap_or(0);
    for (key, value) in [
        ("name", name.to_owned()),
        ("version", "1".to_owned()),
        ("encoding", "terrarium".to_owned()),
        ("format", "png".to_owned()),
        ("minzoom", "0".to_owned()),
        ("maxzoom", max_zoom.to_string()),
        ("tile_size", TILE_SIZE.to_string()),
    ] {
        connection
            .execute(
                "INSERT INTO metadata (name, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .expect("insert fixture metadata");
    }
    for tile in tiles {
        let tms_row = (1u32 << tile.zoom) - 1 - tile.y;
        connection
            .execute(
                "INSERT INTO tiles (zoom_level, tile_column, tile_row, tile_data)
                 VALUES (?1, ?2, ?3, ?4)",
                params![tile.zoom, tile.x, tms_row, terrarium_png(tile.elevation_m)],
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
    let pixels = pixel.repeat((TILE_SIZE * TILE_SIZE) as usize);
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes, TILE_SIZE, TILE_SIZE);
    encoder.set_color(ColorType::Rgb);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write fixture PNG header");
    writer
        .write_image_data(&pixels)
        .expect("write fixture PNG pixels");
    writer.finish().expect("finish fixture PNG");
    bytes
}

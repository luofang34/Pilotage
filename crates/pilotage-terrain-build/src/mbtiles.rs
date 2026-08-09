//! Canonical MBTiles schema and serialization.

use std::collections::BTreeMap;

use rusqlite::{Connection, MAIN_DB, params};

use crate::TerrainBuildError;
use crate::config::TerrainBuildConfig;
use crate::tile::{RasterTile, TILE_SIZE};

const SCHEMA: &str = "
    CREATE TABLE metadata (
        name TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    ) WITHOUT ROWID;
    CREATE TABLE tiles (
        zoom_level INTEGER NOT NULL,
        tile_column INTEGER NOT NULL,
        tile_row INTEGER NOT NULL,
        tile_data BLOB NOT NULL,
        PRIMARY KEY (zoom_level, tile_column, tile_row)
    ) WITHOUT ROWID;
";

pub(crate) fn encode_archive(
    config: TerrainBuildConfig,
    tiles: &[RasterTile],
) -> Result<Vec<u8>, TerrainBuildError> {
    let mut connection = Connection::open_in_memory().map_err(mbtiles_error)?;
    connection
        .execute_batch(
            "PRAGMA page_size = 4096;
             PRAGMA journal_mode = OFF;
             PRAGMA synchronous = OFF;
             PRAGMA auto_vacuum = NONE;",
        )
        .map_err(mbtiles_error)?;
    connection.execute_batch(SCHEMA).map_err(mbtiles_error)?;
    insert_contents(&mut connection, config, tiles)?;
    connection.execute_batch("VACUUM;").map_err(mbtiles_error)?;
    let serialized = connection.serialize(MAIN_DB).map_err(mbtiles_error)?;
    Ok(serialized.to_vec())
}

fn insert_contents(
    connection: &mut Connection,
    config: TerrainBuildConfig,
    tiles: &[RasterTile],
) -> Result<(), TerrainBuildError> {
    let transaction = connection.transaction().map_err(mbtiles_error)?;
    {
        let mut statement = transaction
            .prepare("INSERT INTO metadata (name, value) VALUES (?1, ?2)")
            .map_err(mbtiles_error)?;
        for (name, value) in metadata(config) {
            statement
                .execute(params![name, value])
                .map_err(mbtiles_error)?;
        }
    }
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO tiles
                 (zoom_level, tile_column, tile_row, tile_data)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(mbtiles_error)?;
        for tile in tiles {
            let tms_row = (1u32 << u32::from(tile.zoom)) - 1 - tile.y;
            statement
                .execute(params![tile.zoom, tile.x, tms_row, tile.png])
                .map_err(mbtiles_error)?;
        }
    }
    transaction.commit().map_err(mbtiles_error)
}

fn metadata(config: TerrainBuildConfig) -> BTreeMap<&'static str, String> {
    let region = config.region;
    BTreeMap::from([
        (
            "bounds",
            format!(
                "{},{},{},{}",
                region.min_lon_deg, region.min_lat_deg, region.max_lon_deg, region.max_lat_deg
            ),
        ),
        (
            "description",
            "Cosmetic terrain elevation for dark hillshade".to_owned(),
        ),
        ("encoding", "terrarium".to_owned()),
        ("format", "png".to_owned()),
        ("maxzoom", config.max_zoom.to_string()),
        ("minzoom", config.min_zoom.to_string()),
        ("name", "Pilotage terrain".to_owned()),
        ("tile_size", TILE_SIZE.to_string()),
        ("type", "baselayer".to_owned()),
        ("version", "1".to_owned()),
    ])
}

fn mbtiles_error(source: rusqlite::Error) -> TerrainBuildError {
    TerrainBuildError::MbtilesEncoding { source }
}

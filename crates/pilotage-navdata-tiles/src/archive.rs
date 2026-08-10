//! Canonical MBTiles 1.3 schema and serialization.

use std::collections::BTreeMap;
use std::io::Write;

use flate2::{Compression, GzBuilder};
use pilotage_airspace_view::NavdataIdentityV1;
use rusqlite::{Connection, MAIN_DB, params};

use crate::config::NavdataTileConfig;
use crate::feature::LayerKind;
use crate::mercator::TileCoord;
use crate::mvt::encode_tile;
use crate::tile::VectorTile;
use crate::{NAVDATA_TILE_SCHEMA_VERSION, NavdataTileError};

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

struct TileBlob {
    coord: TileCoord,
    bytes: Vec<u8>,
}

pub(crate) fn encode_archive(
    identity: &NavdataIdentityV1,
    config: NavdataTileConfig,
    tiles: &[VectorTile],
) -> Result<Vec<u8>, NavdataTileError> {
    let blobs = encode_tiles(tiles)?;
    let metadata = metadata(identity, config, tiles)?;
    let mut connection = Connection::open_in_memory().map_err(|source| mbtiles("open", source))?;
    connection
        .execute_batch(
            "PRAGMA page_size = 4096;
             PRAGMA journal_mode = OFF;
             PRAGMA synchronous = OFF;
             PRAGMA auto_vacuum = NONE;",
        )
        .map_err(|source| mbtiles("configure", source))?;
    connection
        .execute_batch(SCHEMA)
        .map_err(|source| mbtiles("create schema", source))?;
    insert_contents(&mut connection, &metadata, &blobs)?;
    connection
        .execute_batch("VACUUM;")
        .map_err(|source| mbtiles("vacuum", source))?;
    let serialized = connection
        .serialize(MAIN_DB)
        .map_err(|source| mbtiles("serialize", source))?;
    Ok(serialized.to_vec())
}

fn encode_tiles(tiles: &[VectorTile]) -> Result<Vec<TileBlob>, NavdataTileError> {
    tiles
        .iter()
        .map(|tile| {
            let encoded = encode_tile(tile)?;
            let bytes = compress(tile.coord, &encoded)?;
            Ok(TileBlob {
                coord: tile.coord,
                bytes,
            })
        })
        .collect()
}

fn compress(coord: TileCoord, bytes: &[u8]) -> Result<Vec<u8>, NavdataTileError> {
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(Vec::new(), Compression::best());
    encoder
        .write_all(bytes)
        .map_err(|source| compression_error(coord, source))?;
    encoder
        .finish()
        .map_err(|source| compression_error(coord, source))
}

fn insert_contents(
    connection: &mut Connection,
    metadata: &BTreeMap<&'static str, String>,
    tiles: &[TileBlob],
) -> Result<(), NavdataTileError> {
    let transaction = connection
        .transaction()
        .map_err(|source| mbtiles("begin transaction", source))?;
    insert_metadata(&transaction, metadata)?;
    insert_tiles(&transaction, tiles)?;
    transaction
        .commit()
        .map_err(|source| mbtiles("commit", source))
}

fn insert_metadata(
    transaction: &rusqlite::Transaction<'_>,
    metadata: &BTreeMap<&'static str, String>,
) -> Result<(), NavdataTileError> {
    let mut statement = transaction
        .prepare("INSERT INTO metadata (name, value) VALUES (?1, ?2)")
        .map_err(|source| mbtiles("prepare metadata", source))?;
    for (name, value) in metadata {
        statement
            .execute(params![name, value])
            .map_err(|source| mbtiles("insert metadata", source))?;
    }
    Ok(())
}

fn insert_tiles(
    transaction: &rusqlite::Transaction<'_>,
    tiles: &[TileBlob],
) -> Result<(), NavdataTileError> {
    let mut statement = transaction
        .prepare(
            "INSERT INTO tiles (zoom_level, tile_column, tile_row, tile_data)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|source| mbtiles("prepare tiles", source))?;
    for tile in tiles {
        let width = 1u32 << u32::from(tile.coord.zoom);
        let tms_row = width.wrapping_sub(1).wrapping_sub(tile.coord.y);
        statement
            .execute(params![tile.coord.zoom, tile.coord.x, tms_row, tile.bytes])
            .map_err(|source| mbtiles("insert tile", source))?;
    }
    Ok(())
}

fn metadata(
    identity: &NavdataIdentityV1,
    config: NavdataTileConfig,
    tiles: &[VectorTile],
) -> Result<BTreeMap<&'static str, String>, NavdataTileError> {
    let layer_json = serde_json::to_string(&VectorMetadata::new(config, tiles))
        .map_err(|source| NavdataTileError::LayerMetadataEncoding { source })?;
    let min_zoom = tiles.iter().map(|tile| tile.coord.zoom).min().unwrap_or(0);
    let max_zoom = tiles.iter().map(|tile| tile.coord.zoom).max().unwrap_or(0);
    Ok(BTreeMap::from([
        (
            "bounds",
            "-180,-85.0511287798066,180,85.0511287798066".to_owned(),
        ),
        (
            "description",
            "Cycle-scoped aeronautical baseline".to_owned(),
        ),
        ("format", "pbf".to_owned()),
        ("json", layer_json),
        ("maxzoom", max_zoom.to_string()),
        ("minzoom", min_zoom.to_string()),
        ("name", format!("Pilotage Navdata {}", identity.cycle)),
        ("pilotage_cycle", identity.cycle.clone()),
        ("pilotage_schema", NAVDATA_TILE_SCHEMA_VERSION.to_string()),
        ("pilotage_snapshot_digest", identity.snapshot_digest.clone()),
        ("pilotage_snapshot_id", identity.snapshot_id.clone()),
        ("type", "baselayer".to_owned()),
        ("version", NAVDATA_TILE_SCHEMA_VERSION.to_string()),
    ]))
}

#[derive(serde::Serialize)]
struct VectorMetadata {
    vector_layers: Vec<VectorLayerMetadata>,
}

impl VectorMetadata {
    fn new(config: NavdataTileConfig, tiles: &[VectorTile]) -> Self {
        Self {
            vector_layers: LayerKind::all()
                .into_iter()
                .filter_map(|layer| {
                    let (minzoom, maxzoom) = layer_zoom_range(tiles, layer)?;
                    Some(VectorLayerMetadata {
                        id: layer.name(),
                        fields: layer_fields(layer),
                        minzoom: minzoom.max(layer.min_zoom(config)),
                        maxzoom: maxzoom.min(config.max_zoom),
                    })
                })
                .collect(),
        }
    }
}

#[derive(serde::Serialize)]
struct VectorLayerMetadata {
    id: &'static str,
    fields: BTreeMap<&'static str, &'static str>,
    minzoom: u8,
    maxzoom: u8,
}

fn layer_zoom_range(tiles: &[VectorTile], layer: LayerKind) -> Option<(u8, u8)> {
    let mut zooms = tiles
        .iter()
        .filter(|tile| {
            tile.layers
                .get(&layer)
                .is_some_and(|features| !features.is_empty())
        })
        .map(|tile| tile.coord.zoom);
    let first = zooms.next()?;
    Some(zooms.fold((first, first), |(minimum, maximum), zoom| {
        (minimum.min(zoom), maximum.max(zoom))
    }))
}

fn layer_fields(layer: LayerKind) -> BTreeMap<&'static str, &'static str> {
    let mut fields = BTreeMap::from([
        ("identifier", "String"),
        ("kind", "String"),
        ("name", "String"),
        ("subject_cycle", "String"),
        ("subject_id", "String"),
    ]);
    match layer {
        LayerKind::Airway => {
            fields.insert("location", "String");
        }
        LayerKind::Airspace => {
            fields.insert("geometry_quality", "String");
            fields.insert("lower", "String");
            fields.insert("upper", "String");
        }
        LayerKind::Aerodrome | LayerKind::Navaid | LayerKind::Fix => {}
    }
    fields
}

fn compression_error(coord: TileCoord, source: std::io::Error) -> NavdataTileError {
    NavdataTileError::TileCompression {
        zoom: coord.zoom,
        x: coord.x,
        y: coord.y,
        source,
    }
}

fn mbtiles(operation: &'static str, source: rusqlite::Error) -> NavdataTileError {
    NavdataTileError::Mbtiles { operation, source }
}

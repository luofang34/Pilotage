//! Offline archive access for clients and verification tools.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use pilotage_airspace_view::NavdataIdentityV1;
use rusqlite::{Connection, MAIN_DB, OptionalExtension, params};

use crate::{NAVDATA_TILE_SCHEMA_VERSION, NavdataTileError};

/// Read-only access to one installed Navdata MBTiles archive.
pub struct OfflineTileReader {
    connection: Connection,
    identity: NavdataIdentityV1,
}

impl OfflineTileReader {
    /// Opens an installed archive without a network.
    ///
    /// # Errors
    ///
    /// Returns [`NavdataTileError`] if the file cannot be read or the archive
    /// schema and identity metadata are not valid.
    pub fn open_file_blocking(path: &Path) -> Result<Self, NavdataTileError> {
        let bytes = fs::read(path).map_err(|source| NavdataTileError::ArchiveRead {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_bytes(&bytes)
    }

    /// Opens complete MBTiles bytes in a read-only in-memory database.
    ///
    /// # Errors
    ///
    /// Returns [`NavdataTileError`] if the bytes are not a supported Navdata
    /// tile bundle.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NavdataTileError> {
        let mut connection =
            Connection::open_in_memory().map_err(|source| mbtiles("open", source))?;
        connection
            .deserialize_read_exact(MAIN_DB, Cursor::new(bytes), bytes.len(), true)
            .map_err(|source| mbtiles("deserialize", source))?;
        connection
            .execute_batch("PRAGMA query_only = ON;")
            .map_err(|source| mbtiles("set read-only", source))?;
        validate_metadata(&connection)?;
        let identity = NavdataIdentityV1 {
            cycle: metadata(&connection, "pilotage_cycle")?,
            snapshot_id: metadata(&connection, "pilotage_snapshot_id")?,
            snapshot_digest: metadata(&connection, "pilotage_snapshot_digest")?,
        };
        Ok(Self {
            connection,
            identity,
        })
    }

    /// Gets the identity carried in the archive.
    #[must_use]
    pub const fn identity(&self) -> &NavdataIdentityV1 {
        &self.identity
    }

    /// Gets a gzip-compressed Mapbox Vector Tile by XYZ coordinate.
    ///
    /// # Errors
    ///
    /// Returns [`NavdataTileError`] when the coordinate is outside its tile
    /// matrix or SQLite cannot read the archive.
    pub fn tile_xyz(&self, zoom: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, NavdataTileError> {
        let Some(width) = 1u32.checked_shl(u32::from(zoom)) else {
            return Err(invalid_coordinate(zoom, x, y));
        };
        if x >= width || y >= width {
            return Err(invalid_coordinate(zoom, x, y));
        }
        let tms_row = width.wrapping_sub(1).wrapping_sub(y);
        self.connection
            .query_row(
                "SELECT tile_data FROM tiles
                 WHERE zoom_level = ?1 AND tile_column = ?2 AND tile_row = ?3",
                params![zoom, x, tms_row],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| mbtiles("read tile", source))
    }
}

fn validate_metadata(connection: &Connection) -> Result<(), NavdataTileError> {
    for (name, expected) in [
        ("format", "pbf"),
        ("pilotage_schema", &NAVDATA_TILE_SCHEMA_VERSION.to_string()),
    ] {
        let value = metadata(connection, name)?;
        if value != expected {
            return Err(NavdataTileError::UnsupportedMetadata { name, value });
        }
    }
    Ok(())
}

fn metadata(connection: &Connection, name: &'static str) -> Result<String, NavdataTileError> {
    connection
        .query_row(
            "SELECT value FROM metadata WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| mbtiles("read metadata", source))?
        .ok_or(NavdataTileError::MissingMetadata { name })
}

fn invalid_coordinate(zoom: u8, x: u32, y: u32) -> NavdataTileError {
    NavdataTileError::InvalidTileCoordinate { zoom, x, y }
}

fn mbtiles(operation: &'static str, source: rusqlite::Error) -> NavdataTileError {
    NavdataTileError::Mbtiles { operation, source }
}

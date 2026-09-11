use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::ArchiveError;

/// A read-only SQLite connection to an installed MBTiles archive.
pub struct MbTiles {
    path: PathBuf,
    connection: Connection,
}

#[cfg(test)]
mod tests;

impl MbTiles {
    /// Open an archive without reading the complete file into memory.
    pub fn open_blocking(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|source| ArchiveError::Mbtiles {
                path: path.clone(),
                source,
            })?;
        connection
            .prepare("SELECT zoom_level, tile_column, tile_row, tile_data FROM tiles LIMIT 0")
            .map_err(|source| ArchiveError::Mbtiles {
                path: path.clone(),
                source,
            })?;
        Ok(Self { path, connection })
    }

    /// Read a tile by XYZ coordinate and convert its row to the MBTiles TMS row.
    pub fn tile_blocking(&self, zoom: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, ArchiveError> {
        let width = 1u32
            .checked_shl(u32::from(zoom))
            .ok_or(ArchiveError::Coordinate { zoom, x, y })?;
        if x >= width || y >= width {
            return Err(ArchiveError::Coordinate { zoom, x, y });
        }
        self.connection.query_row(
            "SELECT tile_data FROM tiles WHERE zoom_level = ?1 AND tile_column = ?2 AND tile_row = ?3",
            params![zoom, x, width - 1 - y], |row| row.get(0),
        ).optional().map_err(|source| ArchiveError::Mbtiles { path: self.path.clone(), source })
    }

    /// Read one metadata value from the archive.
    pub fn metadata_blocking(&self, name: &str) -> Result<Option<String>, ArchiveError> {
        self.connection
            .query_row(
                "SELECT value FROM metadata WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| ArchiveError::Mbtiles {
                path: self.path.clone(),
                source,
            })
    }
}

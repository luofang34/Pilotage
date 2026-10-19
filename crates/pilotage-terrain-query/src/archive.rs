//! Read-only MBTiles access and decoded-tile caching.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::TerrainQueryError;
use crate::tile::{DecodedTile, TileAddress, WEB_MERCATOR_MAX_LAT_DEG, address_for_position};

const TILE_CACHE_CAPACITY: usize = 16;

/// One elevation and the tile level that supplied it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElevationSample {
    /// Terrarium elevation, metres.
    pub elevation_m: f64,
    /// The zoom of the tile that held the pixel.
    pub zoom: u8,
    /// The tile width in pixels.
    pub tile_size: u32,
}

/// A read-only Terrarium MBTiles archive.
pub struct TerrainArchive {
    path: PathBuf,
    connection: Connection,
    min_zoom: u8,
    max_zoom: u8,
    tile_size: u32,
    cache: VecDeque<(TileAddress, DecodedTile)>,
}

impl TerrainArchive {
    /// Open and validate one Terrarium MBTiles archive.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainQueryError`] if SQLite cannot open the file or the
    /// required metadata does not describe 8-bit RGB Terrarium PNG tiles.
    pub fn open_blocking(path: impl AsRef<Path>) -> Result<Self, TerrainQueryError> {
        let path = path.as_ref();
        let archive_path = path.to_path_buf();
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|source| TerrainQueryError::ArchiveOpen {
                path: archive_path.clone(),
                source,
            })?;
        let encoding = metadata(&connection, &archive_path, "encoding")?;
        validate_metadata(&archive_path, "encoding", &encoding, "terrarium")?;
        let format = metadata(&connection, &archive_path, "format")?;
        validate_metadata(&archive_path, "format", &format, "png")?;
        let min_zoom = metadata_u8(&connection, &archive_path, "minzoom")?;
        let max_zoom = metadata_u8(&connection, &archive_path, "maxzoom")?;
        if min_zoom > max_zoom || max_zoom > 30 {
            return Err(TerrainQueryError::UnsupportedMetadata {
                path: archive_path,
                name: "zoom range",
                value: format!("{min_zoom}-{max_zoom}"),
            });
        }
        let tile_size = metadata_u32(&connection, &archive_path, "tile_size")?;
        if tile_size == 0 {
            return Err(TerrainQueryError::UnsupportedMetadata {
                path: archive_path,
                name: "tile_size",
                value: tile_size.to_string(),
            });
        }
        Ok(Self {
            path: archive_path,
            connection,
            min_zoom,
            max_zoom,
            tile_size,
            cache: VecDeque::with_capacity(TILE_CACHE_CAPACITY),
        })
    }

    /// Read elevation at one WGS84 position.
    ///
    /// The result is `None` when the position is outside Web Mercator or no
    /// archive tile covers it. The reader tries less detailed zooms before it
    /// reports no coverage.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainQueryError`] if the position is invalid, SQLite fails,
    /// or a selected tile does not contain an 8-bit RGB PNG.
    pub fn elevation_m_blocking(
        &mut self,
        latitude_deg: f64,
        longitude_deg: f64,
    ) -> Result<Option<f64>, TerrainQueryError> {
        Ok(self
            .elevation_sample_blocking(latitude_deg, longitude_deg)?
            .map(|sample| sample.elevation_m))
    }

    /// Read elevation at one WGS84 position, with the zoom of the tile
    /// that supplied it.
    ///
    /// A sparse archive answers some positions from a less detailed tile;
    /// the zoom tells a caller how coarse that answer is.
    ///
    /// # Errors
    ///
    /// The same errors as [`Self::elevation_m_blocking`].
    pub fn elevation_sample_blocking(
        &mut self,
        latitude_deg: f64,
        longitude_deg: f64,
    ) -> Result<Option<ElevationSample>, TerrainQueryError> {
        validate_position(latitude_deg, longitude_deg)?;
        if !(-WEB_MERCATOR_MAX_LAT_DEG..=WEB_MERCATOR_MAX_LAT_DEG).contains(&latitude_deg) {
            return Ok(None);
        }
        for zoom in (self.min_zoom..=self.max_zoom).rev() {
            let (address, pixel) =
                address_for_position(zoom, latitude_deg, longitude_deg, self.tile_size);
            if let Some(elevation_m) = self.cached_elevation(address, pixel)? {
                return Ok(Some(ElevationSample {
                    elevation_m,
                    zoom,
                    tile_size: self.tile_size,
                }));
            }
        }
        Ok(None)
    }

    /// The archive's `name` metadata and its `version` when it states
    /// one: together they identify the terrain it holds.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainQueryError::ArchiveRead`] if `name` is absent or
    /// SQLite fails.
    pub fn identity_blocking(&self) -> Result<(String, Option<String>), TerrainQueryError> {
        let name = metadata(&self.connection, &self.path, "name")?;
        let version = self
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE name = 'version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| TerrainQueryError::ArchiveRead {
                path: self.path.clone(),
                context: "metadata version".to_owned(),
                source,
            })?;
        Ok((name, version))
    }

    fn cached_elevation(
        &mut self,
        address: TileAddress,
        pixel: crate::tile::PixelAddress,
    ) -> Result<Option<f64>, TerrainQueryError> {
        if let Some(index) = self.cache.iter().position(|(key, _)| *key == address) {
            let Some(entry) = self.cache.remove(index) else {
                return Ok(None);
            };
            let elevation = entry.1.elevation_m(pixel);
            self.cache.push_front(entry);
            return Ok(elevation);
        }
        let Some(bytes) = tile_bytes(&self.connection, &self.path, address)? else {
            return Ok(None);
        };
        let tile = DecodedTile::decode(address, &bytes, self.tile_size, &self.path)?;
        let elevation = tile.elevation_m(pixel);
        self.cache.push_front((address, tile));
        self.cache.truncate(TILE_CACHE_CAPACITY);
        Ok(elevation)
    }
}

fn tile_bytes(
    connection: &Connection,
    archive_path: &Path,
    address: TileAddress,
) -> Result<Option<Vec<u8>>, TerrainQueryError> {
    let tms_row = (1u32 << u32::from(address.zoom)) - 1 - address.y;
    connection
        .query_row(
            "SELECT tile_data FROM tiles
             WHERE zoom_level = ?1 AND tile_column = ?2 AND tile_row = ?3",
            params![address.zoom, address.x, tms_row],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| TerrainQueryError::ArchiveRead {
            path: archive_path.to_path_buf(),
            context: format!("tile {}/{}/{}", address.zoom, address.x, address.y),
            source,
        })
}

fn metadata(
    connection: &Connection,
    archive_path: &Path,
    name: &'static str,
) -> Result<String, TerrainQueryError> {
    connection
        .query_row(
            "SELECT value FROM metadata WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .map_err(|source| TerrainQueryError::ArchiveRead {
            path: archive_path.to_path_buf(),
            context: format!("metadata {name}"),
            source,
        })
}

fn metadata_u8(
    connection: &Connection,
    archive_path: &Path,
    name: &'static str,
) -> Result<u8, TerrainQueryError> {
    let value = metadata(connection, archive_path, name)?;
    value
        .parse()
        .map_err(|_| TerrainQueryError::UnsupportedMetadata {
            path: archive_path.to_path_buf(),
            name,
            value,
        })
}

fn metadata_u32(
    connection: &Connection,
    archive_path: &Path,
    name: &'static str,
) -> Result<u32, TerrainQueryError> {
    let value = metadata(connection, archive_path, name)?;
    value
        .parse()
        .map_err(|_| TerrainQueryError::UnsupportedMetadata {
            path: archive_path.to_path_buf(),
            name,
            value,
        })
}

fn validate_metadata(
    archive_path: &Path,
    name: &'static str,
    value: &str,
    required: &str,
) -> Result<(), TerrainQueryError> {
    if value == required {
        Ok(())
    } else {
        Err(TerrainQueryError::UnsupportedMetadata {
            path: archive_path.to_path_buf(),
            name,
            value: value.to_owned(),
        })
    }
}

fn validate_position(latitude_deg: f64, longitude_deg: f64) -> Result<(), TerrainQueryError> {
    let valid = latitude_deg.is_finite()
        && longitude_deg.is_finite()
        && (-90.0..=90.0).contains(&latitude_deg)
        && (-180.0..=180.0).contains(&longitude_deg);
    if valid {
        Ok(())
    } else {
        Err(TerrainQueryError::InvalidPosition {
            latitude_deg,
            longitude_deg,
        })
    }
}

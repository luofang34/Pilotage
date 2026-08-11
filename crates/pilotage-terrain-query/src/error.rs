//! Terrain query errors.

use std::path::PathBuf;

/// A Terrarium MBTiles query failed.
#[derive(Debug, thiserror::Error)]
pub enum TerrainQueryError {
    /// SQLite could not open the archive.
    #[error("terrain archive {path:?} could not be opened")]
    ArchiveOpen {
        /// Archive path.
        path: PathBuf,
        /// SQLite failure.
        #[source]
        source: rusqlite::Error,
    },
    /// SQLite could not read required archive data.
    #[error("terrain archive {path:?} query failed for {context}")]
    ArchiveRead {
        /// Archive path.
        path: PathBuf,
        /// Data that the query tried to read.
        context: String,
        /// SQLite failure.
        #[source]
        source: rusqlite::Error,
    },
    /// The archive metadata is not valid for terrain queries.
    #[error("terrain archive {path:?} metadata {name} has unsupported value {value}")]
    UnsupportedMetadata {
        /// Archive path.
        path: PathBuf,
        /// Metadata key.
        name: &'static str,
        /// Rejected value.
        value: String,
    },
    /// The WGS84 position is not finite or is outside its valid range.
    #[error("terrain position ({latitude_deg}, {longitude_deg}) is invalid")]
    InvalidPosition {
        /// Latitude in degrees.
        latitude_deg: f64,
        /// Longitude in degrees.
        longitude_deg: f64,
    },
    /// A Terrarium tile could not be decoded.
    #[error("terrain archive {path:?} tile {zoom}/{x}/{y} is not a valid PNG")]
    TileDecode {
        /// Archive path.
        path: PathBuf,
        /// Web Mercator zoom.
        zoom: u8,
        /// Web Mercator tile column.
        x: u32,
        /// Web Mercator XYZ tile row.
        y: u32,
        /// PNG failure.
        #[source]
        source: png::DecodingError,
    },
    /// A decoded tile does not use the required image layout.
    #[error("terrain archive {path:?} tile {zoom}/{x}/{y} has unsupported image layout {layout}")]
    UnsupportedTile {
        /// Archive path.
        path: PathBuf,
        /// Web Mercator zoom.
        zoom: u8,
        /// Web Mercator tile column.
        x: u32,
        /// Web Mercator XYZ tile row.
        y: u32,
        /// Image layout detail.
        layout: String,
    },
    /// The PNG decoder cannot allocate the required image buffer.
    #[error("terrain archive {path:?} tile {zoom}/{x}/{y} is too large to decode")]
    TileTooLarge {
        /// Archive path.
        path: PathBuf,
        /// Web Mercator zoom.
        zoom: u8,
        /// Web Mercator tile column.
        x: u32,
        /// Web Mercator XYZ tile row.
        y: u32,
    },
}

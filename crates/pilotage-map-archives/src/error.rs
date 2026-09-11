use std::path::PathBuf;

/// An installed map archive could not supply a tile.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// A file operation failed.
    #[error("map archive file operation failed at {path}")]
    Io {
        /// Archive path.
        path: PathBuf,
        /// Underlying file error.
        #[source]
        source: std::io::Error,
    },
    /// PMTiles could not decode an archive or tile.
    #[error("PMTiles operation failed at {path}")]
    Pmtiles {
        /// Archive path.
        path: PathBuf,
        /// Underlying format error.
        #[source]
        source: pmtiles::PmtError,
    },
    /// SQLite could not read an MBTiles archive.
    #[error("MBTiles operation failed at {path}")]
    Mbtiles {
        /// Archive path.
        path: PathBuf,
        /// Underlying SQLite error.
        #[source]
        source: rusqlite::Error,
    },
    /// The coordinate is outside its tile matrix.
    #[error("invalid tile coordinate {zoom}/{x}/{y}")]
    Coordinate {
        /// Zoom level.
        zoom: u8,
        /// Tile column.
        x: u32,
        /// XYZ tile row.
        y: u32,
    },
    /// The header is invalid or requires an unsupported feature.
    #[error("unsupported map archive at {path}: {reason}")]
    Header {
        /// Archive path.
        path: PathBuf,
        /// Rejected property.
        reason: &'static str,
    },
}

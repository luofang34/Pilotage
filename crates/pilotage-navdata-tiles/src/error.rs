//! Typed failures for Navdata tile construction and access.

use std::io;
use std::path::PathBuf;

/// Failure to build or read a Navdata tile bundle.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NavdataTileError {
    /// A tile scale is invalid.
    #[error("invalid Navdata tile configuration: {reason}")]
    InvalidConfig {
        /// Configuration defect.
        reason: String,
    },
    /// A required snapshot identity field is empty.
    #[error("Navdata identity field {field} is empty")]
    EmptyIdentity {
        /// Empty field name.
        field: &'static str,
    },
    /// A coordinate is not valid WGS84 input.
    #[error("feature {identifier} has invalid WGS84 coordinate ({latitude}, {longitude})")]
    InvalidCoordinate {
        /// Published feature identifier.
        identifier: String,
        /// Latitude in degrees.
        latitude: f64,
        /// Longitude in degrees.
        longitude: f64,
    },
    /// Mapbox Vector Tile encoding failed.
    #[error("could not encode tile {zoom}/{x}/{y}")]
    VectorTileEncoding {
        /// Tile zoom.
        zoom: u8,
        /// Tile column.
        x: u32,
        /// XYZ tile row.
        y: u32,
        /// Protocol Buffer failure.
        #[source]
        source: prost::EncodeError,
    },
    /// Gzip compression failed.
    #[error("could not compress tile {zoom}/{x}/{y}")]
    TileCompression {
        /// Tile zoom.
        zoom: u8,
        /// Tile column.
        x: u32,
        /// XYZ tile row.
        y: u32,
        /// Compression I/O failure.
        #[source]
        source: io::Error,
    },
    /// Vector layer metadata encoding failed.
    #[error("could not encode vector layer metadata")]
    LayerMetadataEncoding {
        /// JSON failure.
        #[source]
        source: serde_json::Error,
    },
    /// SQLite MBTiles construction or access failed.
    #[error("MBTiles operation failed during {operation}")]
    Mbtiles {
        /// Operation that failed.
        operation: &'static str,
        /// SQLite failure.
        #[source]
        source: rusqlite::Error,
    },
    /// An installed archive could not be read.
    #[error("could not read Navdata tile bundle {path}")]
    ArchiveRead {
        /// Archive path.
        path: PathBuf,
        /// File read failure.
        #[source]
        source: io::Error,
    },
    /// The archive does not contain required metadata.
    #[error("Navdata tile bundle has no {name} metadata")]
    MissingMetadata {
        /// Missing metadata key.
        name: &'static str,
    },
    /// The archive metadata has an unsupported value.
    #[error("Navdata tile bundle metadata {name} has unsupported value {value}")]
    UnsupportedMetadata {
        /// Metadata key.
        name: &'static str,
        /// Unsupported value.
        value: String,
    },
    /// An XYZ tile coordinate is outside its zoom matrix.
    #[error("tile coordinate {zoom}/{x}/{y} is outside its zoom matrix")]
    InvalidTileCoordinate {
        /// Tile zoom.
        zoom: u8,
        /// Tile column.
        x: u32,
        /// XYZ tile row.
        y: u32,
    },
}

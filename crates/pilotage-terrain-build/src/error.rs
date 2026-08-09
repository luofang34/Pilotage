//! Typed errors for the cosmetic terrain build.

/// A failure that prevents the builder from making an archive.
#[derive(Debug, thiserror::Error)]
pub enum TerrainBuildError {
    /// A configuration value is invalid.
    #[error("invalid terrain build configuration: {reason}")]
    InvalidConfig {
        /// The configuration rule that failed.
        reason: &'static str,
    },
    /// The source dataset has no terrain grid.
    #[error("source dataset has no terrain grids")]
    EmptyTerrain,
    /// A terrain grid has no source metadata.
    #[error("terrain source {source_id} has no source metadata")]
    MissingSourceMetadata {
        /// Source identifier of the grid.
        source_id: u32,
    },
    /// A terrain grid has more than one source metadata record.
    #[error("terrain source {source_id} has duplicate source metadata")]
    DuplicateSourceMetadata {
        /// Source identifier that occurs more than once.
        source_id: u32,
    },
    /// A source datum is unknown or is not valid for Terrarium output.
    #[error("terrain source {source_id} has unsupported {axis} datum code {code}")]
    UnsupportedSourceDatum {
        /// Source identifier of the grid.
        source_id: u32,
        /// Datum axis.
        axis: &'static str,
        /// Datum wire code.
        code: u8,
    },
    /// A source datum does not have a necessary identity.
    #[error("terrain source {source_id} has an incomplete datum: {reason}")]
    IncompleteSourceDatum {
        /// Source identifier of the grid.
        source_id: u32,
        /// Missing datum identity.
        reason: &'static str,
    },
    /// A terrain grid cannot be sampled.
    #[error("terrain source {source_id} has an invalid grid: {reason}")]
    InvalidTerrainGrid {
        /// Source identifier of the grid.
        source_id: u32,
        /// The grid rule that failed.
        reason: &'static str,
    },
    /// A source coordinate or height conversion failed.
    #[error("terrain source {source_id} datum conversion failed")]
    DatumConversion {
        /// Source identifier of the grid.
        source_id: u32,
        /// Conversion failure from the source model.
        #[source]
        source: pilotage_svs_build::BuildError,
    },
    /// The source does not cover a selected output pixel.
    #[error("terrain has no elevation at tile {zoom}/{x}/{y} pixel {pixel_x},{pixel_y}")]
    MissingElevation {
        /// Web Mercator zoom.
        zoom: u8,
        /// Web Mercator tile column.
        x: u32,
        /// Web Mercator tile row.
        y: u32,
        /// Pixel column.
        pixel_x: u16,
        /// Pixel row.
        pixel_y: u16,
    },
    /// An elevation is outside the Terrarium numeric range.
    #[error("elevation {elevation_m} m is outside the Terrarium range")]
    ElevationOutsideTerrarium {
        /// Elevation that cannot be encoded.
        elevation_m: f64,
    },
    /// The selected zooms and region make too many tiles.
    #[error("terrain build selects {count} tiles; the limit is {limit}")]
    TooManyTiles {
        /// Selected tile count.
        count: u64,
        /// Fixed tile-count limit.
        limit: u64,
    },
    /// PNG encoding failed for one tile.
    #[error("Terrarium PNG encoding failed for tile {zoom}/{x}/{y}")]
    PngEncoding {
        /// Web Mercator zoom.
        zoom: u8,
        /// Web Mercator tile column.
        x: u32,
        /// Web Mercator tile row.
        y: u32,
        /// PNG encoder error.
        #[source]
        source: png::EncodingError,
    },
    /// SQLite could not make the MBTiles archive.
    #[error("MBTiles encoding failed")]
    MbtilesEncoding {
        /// SQLite error.
        #[source]
        source: rusqlite::Error,
    },
}

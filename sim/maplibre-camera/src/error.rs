//! Typed failures of the camera producer.

use std::path::PathBuf;

use thiserror::Error;

/// Why the camera producer could not start or keep running.
#[derive(Debug, Error)]
pub enum CameraError {
    /// The command line could not be parsed.
    #[error("invalid arguments: {0}")]
    Args(#[from] crate::args::ArgsError),
    /// The style file could not be read.
    #[error("failed to read style {path}: {source}")]
    StyleRead {
        /// The style path.
        path: PathBuf,
        /// The read failure.
        #[source]
        source: std::io::Error,
    },
    /// The style is not a MapLibre style document.
    #[error("style {origin} does not parse: {source}")]
    StyleParse {
        /// Where the style came from.
        origin: String,
        /// The parse failure.
        #[source]
        source: serde_json::Error,
    },
    /// No cache directory could be resolved for map tiles.
    #[error("no cache directory for map tiles; pass --cache-dir")]
    NoCacheDir,
    /// A cache path is not UTF-8, which the tile cache needs.
    #[error("cache directory {0} is not valid UTF-8")]
    CachePath(PathBuf),
    /// A calibration directory could not be written.
    #[error("failed to write calibration {path}: {source}")]
    CalibrationWrite {
        /// The artifact path.
        path: PathBuf,
        /// The write failure.
        #[source]
        source: std::io::Error,
    },
    /// A camera's calibration failed its own validation.
    #[error("camera {camera_id} detent {detent} calibration is invalid: {source:?}")]
    Calibration {
        /// The camera id.
        camera_id: u32,
        /// The zoom detent.
        detent: usize,
        /// The validation failure.
        source: pilotage_camera_calibration::CalibrationError,
    },
    /// The offscreen renderer could not be created.
    #[error("camera {camera_id} renderer could not be created: {source}")]
    Renderer {
        /// The camera id.
        camera_id: u32,
        /// The renderer failure.
        #[source]
        source: maplibre::headless::HeadlessRendererError,
    },
    /// The map could not be built from the style.
    #[error("camera {camera_id} map could not be built: {source}")]
    Map {
        /// The camera id.
        camera_id: u32,
        /// The map failure.
        #[source]
        source: maplibre::map::MapError,
    },
    /// The async runtime could not be built.
    #[error("failed to build the async runtime: {0}")]
    Runtime(#[source] std::io::Error),
    /// The render thread could not be started.
    #[error("failed to start the render thread: {0}")]
    RenderThread(#[source] std::io::Error),
    /// A camera's renderer or map could not be built; the render thread
    /// logged the cause.
    #[error("a camera could not be set up")]
    CameraSetup,
    /// The render thread ended with a panic.
    #[error("the render thread panicked")]
    RenderThreadPanicked,
}

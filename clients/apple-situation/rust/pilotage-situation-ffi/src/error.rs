//! Errors that cross the Apple FFI boundary.

/// A facade operation failed.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    /// A track record did not match the linked schema.
    #[error("track record is invalid: {message}")]
    TrackRecord {
        /// Validation or decode detail.
        message: String,
    },
    /// A weather record did not match the linked schema.
    #[error("weather record is invalid: {message}")]
    WeatherRecord {
        /// Validation or decode detail.
        message: String,
    },
    /// Display-value conversion failed.
    #[error("display conversion failed: {message}")]
    Presentation {
        /// Conversion detail.
        message: String,
    },
    /// The session state is not available.
    #[error("session state is not available: {message}")]
    SessionState {
        /// Lock failure detail.
        message: String,
    },
    /// The linked producer supplied a delta that this facade does not know.
    #[error("track delta kind is not supported by this facade")]
    UnsupportedTrackDelta,
}

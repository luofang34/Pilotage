//! Errors that cross the Apple FFI boundary.

/// A facade operation failed.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    /// The radio-domain state could not be created.
    #[error("radio-domain state is not available: {message}")]
    RadioState {
        /// Configuration detail.
        message: String,
    },
    /// One batch of AeroLink reception events could not be processed.
    #[error("radio input failed: {message}")]
    RadioInput {
        /// Decode, adapter, store, or lifecycle detail.
        message: String,
    },
    /// A weather station position is invalid.
    #[error("weather station position is invalid: {message}")]
    WeatherStationPosition {
        /// Station and coordinate detail.
        message: String,
    },
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
    /// A layer identity is not in the portable catalog.
    #[error("display layer is not known: {layer_id}")]
    UnknownLayer {
        /// Rejected layer identity.
        layer_id: String,
    },
    /// The session state is not available.
    #[error("session state is not available: {message}")]
    SessionState {
        /// Lock failure detail.
        message: String,
    },
}

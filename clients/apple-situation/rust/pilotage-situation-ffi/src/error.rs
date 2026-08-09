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
    /// The session state is not available.
    #[error("session state is not available: {message}")]
    SessionState {
        /// Lock failure detail.
        message: String,
    },
}

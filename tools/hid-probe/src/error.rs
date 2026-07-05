//! Typed errors for the `hid-probe` binary.

use std::path::PathBuf;

/// Errors this tool's subcommands can produce.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// Argument parsing failed: an unknown subcommand or missing/malformed
    /// flag value.
    #[error("usage error: {message}")]
    Usage {
        /// Human-readable description of what was wrong with the arguments.
        message: String,
    },
    /// The underlying `hidapi` backend failed to initialize, enumerate, or
    /// open a device.
    #[error("hidapi error: {message}")]
    Hid {
        /// Message from the underlying `hidapi::HidError`.
        message: String,
    },
    /// Writing the capture JSON file to disk failed.
    #[error("failed to write capture file {path}: {source}")]
    CaptureWrite {
        /// Path the capture was being written to.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Serializing the capture to JSON failed.
    #[error("failed to serialize capture: {source}")]
    CaptureSerialize {
        /// Underlying serde_json error.
        #[source]
        source: serde_json::Error,
    },
}

impl From<hidapi::HidError> for ProbeError {
    fn from(source: hidapi::HidError) -> Self {
        Self::Hid {
            message: source.to_string(),
        }
    }
}

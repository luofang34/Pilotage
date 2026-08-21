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
    /// A HID report does not match the target device layout.
    #[error("HID report length {actual} does not match required length {expected}")]
    InvalidReportLength {
        /// The observed report length.
        actual: usize,
        /// The required report length.
        expected: usize,
    },
    /// Writing the capture JSON file to disk failed.
    #[error("failed to write capture file {path:?}: {source}")]
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
    /// Reading an input artifact failed.
    #[error("failed to read {path:?}: {source}")]
    ArtifactRead {
        /// Path to the input artifact.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// An input JSON artifact did not match its schema.
    #[error("failed to parse {path:?}: {source}")]
    ArtifactParse {
        /// Path to the input artifact.
        path: PathBuf,
        /// Underlying JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// Canonical candidate digest generation failed.
    #[error("failed to create canonical candidate digest: {source}")]
    CandidateDigest {
        /// The candidate serialization error.
        #[source]
        source: pilotage_input::CharacterizationError,
    },
    /// A device profile failed validation.
    #[error("invalid device profile: {source}")]
    Profile {
        /// The profile validation error.
        #[source]
        source: pilotage_input::ProfileError,
    },
    /// Capture evidence was incomplete or inconsistent.
    #[error("invalid characterization capture: {detail}")]
    InvalidCapture {
        /// The failed invariant.
        detail: String,
    },
    /// A named movement did not identify one physical axis.
    #[error(
        "movement {logical} is not unique: selected axis {source_index}, cross-axis ratio {coupling}"
    )]
    AmbiguousMovement {
        /// The named physical control.
        logical: String,
        /// The strongest source axis.
        source_index: usize,
        /// The largest cross-axis coupling ratio.
        coupling: f32,
    },
    /// A centered physical axis did not move significantly on both sides.
    #[error("movement {logical} on source axis {source_index} did not cover both sides of center")]
    IncompleteMovement {
        /// The named physical control.
        logical: String,
        /// The selected source axis.
        source_index: usize,
    },
    /// Two named controls selected the same source axis.
    #[error(
        "movements {first_logical} and {second_logical} both select source axis {source_index}"
    )]
    DuplicateMovement {
        /// The first named control.
        first_logical: String,
        /// The second named control.
        second_logical: String,
        /// The repeated source axis.
        source_index: usize,
    },
    /// Candidate promotion failed.
    #[error("calibration candidate promotion failed: {source}")]
    Promotion {
        /// The promotion error.
        #[source]
        source: pilotage_input::CharacterizationError,
    },
}

impl From<hidapi::HidError> for ProbeError {
    fn from(source: hidapi::HidError) -> Self {
        Self::Hid {
            message: source.to_string(),
        }
    }
}

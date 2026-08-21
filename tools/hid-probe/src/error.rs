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
    /// The target USB identity did not resolve to one device handle.
    #[error("expected one target HID device, found {found}")]
    TargetDeviceCount {
        /// The number of matching device handles.
        found: usize,
    },
    /// The system clock could not create a capture connection epoch.
    #[error("failed to create a device connection epoch: {source}")]
    Clock {
        /// The system clock error.
        #[source]
        source: std::time::SystemTimeError,
    },
    /// The built-in raw report layout is invalid.
    #[error("the built-in source-axis contract has an invalid report layout: {source}")]
    SourceContractLayout {
        /// The raw report layout error.
        #[source]
        source: pilotage_input::RawReportError,
    },
    /// A captured raw report does not match the built-in layout.
    #[error("a captured raw report does not match the source-axis contract: {source}")]
    RawReportDecode {
        /// The raw report decode error.
        #[source]
        source: pilotage_input::RawReportError,
    },
    /// The HID product name is outside the shared evidence limit.
    #[error("the HID product name is empty or larger than {limit} UTF-8 bytes")]
    ProductNameLimit {
        /// The maximum product name byte count.
        limit: usize,
    },
    /// A native capture reached the shared report limit.
    #[error("characterization capture reached its {limit}-report limit")]
    CaptureSampleLimit {
        /// The maximum report count.
        limit: usize,
    },
    /// An encoded native capture exceeds the shared artifact limit.
    #[error("encoded characterization capture is {actual} bytes; the limit is {limit}")]
    CaptureByteLimit {
        /// The encoded byte count.
        actual: usize,
        /// The maximum encoded byte count.
        limit: usize,
    },
    /// A native capture would retain more than its processing-memory limit.
    #[error("characterization capture would retain {actual} bytes; the limit is {limit}")]
    CaptureMemoryLimit {
        /// The projected retained byte count.
        actual: usize,
        /// The maximum retained byte count.
        limit: usize,
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
    /// Streaming a capture to its create-new artifact failed.
    #[error("failed to stream capture JSON to {path:?}: {source}")]
    CaptureStream {
        /// The capture output path.
        path: PathBuf,
        /// The JSON or output-stream error.
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
    /// An artifact exceeds its command-specific byte limit.
    #[error("artifact {path:?} is larger than {limit} bytes")]
    ArtifactTooLarge {
        /// The rejected artifact path.
        path: PathBuf,
        /// The maximum accepted byte count.
        limit: usize,
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
    /// The built-in source-axis contract did not match its schema.
    #[error("failed to parse the built-in source-axis contract: {source}")]
    SourceContractParse {
        /// The JSON parser error.
        #[source]
        source: serde_json::Error,
    },
    /// Exact characterization evidence failed validation or analysis.
    #[error("HID characterization failed: {source}")]
    Characterization {
        /// The pure analysis error.
        #[source]
        source: pilotage_input::AnalysisError,
    },
    /// Canonical candidate digest generation failed.
    #[error("failed to create canonical candidate digest: {source}")]
    CandidateDigest {
        /// The candidate serialization error.
        #[source]
        source: pilotage_input::CharacterizationError,
    },
    /// Capture evidence was incomplete or inconsistent.
    #[error("invalid characterization capture: {detail}")]
    InvalidCapture {
        /// The failed invariant.
        detail: String,
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

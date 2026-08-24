//! Typed failures from deterministic HID characterization.

use crate::ProfileError;

/// Errors from exact-capture characterization.
#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    /// The exact capture exceeds the processing input limit.
    #[error("characterization capture is {actual} bytes; the limit is {limit}")]
    CaptureTooLarge {
        /// The supplied byte count.
        actual: usize,
        /// The maximum accepted byte count.
        limit: usize,
    },
    /// The exact source-axis contract bytes are not valid JSON.
    #[error("failed to parse the source-axis contract: {source}")]
    ContractParse {
        /// The JSON parser error.
        #[source]
        source: serde_json::Error,
    },
    /// The exact capture bytes are not valid JSON.
    #[error("failed to parse the characterization capture: {source}")]
    CaptureParse {
        /// The JSON parser error.
        #[source]
        source: serde_json::Error,
    },
    /// The baseline profile is invalid.
    #[error("failed to parse the baseline device profile: {source}")]
    Profile {
        /// The profile parser error.
        #[source]
        source: ProfileError,
    },
    /// The source-axis contract and capture do not have the same lineage.
    #[error("source-axis contract mismatch: {detail}")]
    ContractMismatch {
        /// The failed contract invariant.
        detail: String,
    },
    /// The exact contract bytes do not match the digest in the capture.
    #[error("source-axis contract digest {actual} does not match capture digest {expected}")]
    ContractDigestMismatch {
        /// The digest of the supplied exact contract bytes.
        actual: String,
        /// The digest recorded in the capture.
        expected: String,
    },
    /// Capture evidence is incomplete, inconsistent, or outside a limit.
    #[error("invalid characterization capture: {detail}")]
    InvalidCapture {
        /// The failed capture invariant.
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
        /// The largest normalized cross-axis coupling ratio.
        coupling: f32,
    },
    /// A physical control did not reach its trusted source range.
    #[error("movement {logical} on source axis {source_index} did not cover its trusted range")]
    IncompleteMovement {
        /// The named physical control.
        logical: String,
        /// The selected source axis.
        source_index: usize,
    },
    /// Two named movements selected the same source axis.
    #[error(
        "movements {first_logical} and {second_logical} both select source axis {source_index}"
    )]
    DuplicateMovement {
        /// The first named physical control.
        first_logical: String,
        /// The second named physical control.
        second_logical: String,
        /// The repeated source axis.
        source_index: usize,
    },
}

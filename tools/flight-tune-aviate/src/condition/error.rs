//! Why one uncertainty condition cannot launch, arm, or stay armed.

use std::path::PathBuf;

/// A condition launch, handshake, or trace failure.
#[derive(Debug, thiserror::Error)]
pub enum AviateConditionError {
    /// The backend cannot execute the requested uncertainty.
    #[error("condition {condition} is not executable on this backend: {source}")]
    Unsupported {
        /// The condition-set identifier.
        condition: String,
        /// The capability contract failure.
        #[source]
        source: flight_tune::ValidationError,
    },
    /// The condition or its declaration is not valid.
    #[error("condition evidence refused: {source}")]
    Evidence {
        /// The contract failure.
        #[source]
        source: flight_tune::TuneError,
    },
    /// The canonical artifact could not be encoded.
    #[error("condition artifact could not be encoded: {source}")]
    Encode {
        /// The encoding failure.
        #[source]
        source: flight_tune::CodecError,
    },
    /// A file operation on the artifact or its directory failed.
    #[error("{operation} failed for {path}: {source}")]
    Artifact {
        /// The attempted operation.
        operation: &'static str,
        /// The path the operation named.
        path: PathBuf,
        /// The operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// A trace socket operation failed.
    #[error("{operation} failed on the condition trace path: {source}")]
    Trace {
        /// The attempted operation.
        operation: &'static str,
        /// The operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// A trace frame could not be encoded or decoded.
    #[error("a condition trace frame is not readable: {source}")]
    Frame {
        /// The serialization failure.
        #[source]
        source: serde_json::Error,
    },
    /// A trace frame is larger than the protocol permits.
    #[error("a condition trace frame states {bytes} bytes")]
    FrameTooLarge {
        /// The stated frame size.
        bytes: usize,
    },
    /// The executor spoke a protocol this launch does not speak.
    #[error("the executor trace protocol is not the launched one: {detail}")]
    Protocol {
        /// The refusal detail.
        detail: String,
    },
    /// The executor returned identities other than the launched ones.
    #[error("the executor returned other run identities: {detail}")]
    Identity {
        /// The refusal detail.
        detail: String,
    },
    /// A sample does not state the decision the declaration required.
    #[error("the executed uncertainty relation failed: {source}")]
    Relation {
        /// The relation failure.
        #[source]
        source: flight_tune::TuneError,
    },
}

impl AviateConditionError {
    pub(crate) fn protocol(detail: impl Into<String>) -> Self {
        Self::Protocol {
            detail: detail.into(),
        }
    }

    pub(crate) fn identity(detail: impl Into<String>) -> Self {
        Self::Identity {
            detail: detail.into(),
        }
    }

    pub(crate) fn trace(operation: &'static str, source: std::io::Error) -> Self {
        Self::Trace { operation, source }
    }
}

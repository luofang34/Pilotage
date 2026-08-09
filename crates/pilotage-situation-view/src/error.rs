//! Typed contract and corpus errors.

/// Error from request validation, snapshot capture, or corpus verification.
#[derive(Debug, thiserror::Error)]
pub enum SituationViewError {
    /// The request uses an unsupported schema version.
    #[error("SituationView schema version {found} is not supported; expected {expected}")]
    UnsupportedSchemaVersion {
        /// Version in the request.
        found: u16,
        /// Version supported by this implementation.
        expected: u16,
    },
    /// A UTC instant has an invalid subsecond value.
    #[error("{location} has invalid subsecond nanoseconds {subsecond_nanoseconds}")]
    InvalidUtcInstant {
        /// Location of the invalid instant.
        location: String,
        /// Invalid subsecond value.
        subsecond_nanoseconds: u32,
    },
    /// The query selects one domain subject more than once.
    #[error("domain {domain} subject {subject} is selected more than once")]
    DuplicateDomainSelection {
        /// Stable domain name.
        domain: String,
        /// Domain-owned subject identity.
        subject: String,
    },
    /// The query uses one requirement ID more than once.
    #[error("requirement ID {requirement_id} is used more than once")]
    DuplicateRequirementId {
        /// Duplicate caller-owned requirement ID.
        requirement_id: String,
    },
    /// A source returned a snapshot for a different selection.
    #[error(
        "capture for {selected_domain}/{selected_subject} returned {actual_domain}/{actual_subject}"
    )]
    SnapshotSelectionMismatch {
        /// Selected domain name.
        selected_domain: String,
        /// Selected subject identity.
        selected_subject: String,
        /// Captured domain name.
        actual_domain: String,
        /// Captured subject identity.
        actual_subject: String,
    },
    /// The corpus JSON cannot be decoded.
    #[error("SituationView corpus cannot be decoded: {source}")]
    CorpusDecode {
        /// JSON decoding error.
        #[source]
        source: serde_json::Error,
    },
    /// The corpus uses an unsupported version.
    #[error("SituationView corpus version {found} is not supported; expected {expected}")]
    UnsupportedCorpusVersion {
        /// Version in the corpus.
        found: u16,
        /// Version supported by this implementation.
        expected: u16,
    },
    /// A corpus case contains one capture more than once.
    #[error("corpus case {case_name} repeats capture {domain}/{subject}")]
    DuplicateCorpusCapture {
        /// Corpus case name.
        case_name: String,
        /// Stable domain name.
        domain: String,
        /// Domain-owned subject identity.
        subject: String,
    },
    /// An implementation result differs from the required corpus result.
    #[error("SituationView corpus case {case_name} produced a different result")]
    CorpusMismatch {
        /// Corpus case name.
        case_name: String,
    },
}

use std::path::PathBuf;

use thiserror::Error;

use crate::{AdapterError, Digest, EvaluatorError};

/// An error from tuner validation, execution, or storage.
#[derive(Debug, Error)]
pub enum TuneError {
    /// A candidate is not valid for the active stage.
    #[error("candidate validation failed: {detail}")]
    InvalidCandidate {
        /// The validation detail.
        detail: String,
    },
    /// A search stage is not valid.
    #[error("stage validation failed: {detail}")]
    InvalidStage {
        /// The validation detail.
        detail: String,
    },
    /// An artifact or runtime identity is not valid.
    #[error("identity validation failed: {detail}")]
    InvalidIdentity {
        /// The validation detail.
        detail: String,
    },
    /// A hard gate, metric, aggregate, or paired comparison is not valid.
    #[error("score validation failed: {detail}")]
    InvalidScore {
        /// The validation detail.
        detail: String,
    },
    /// An operation is not valid in the current campaign phase.
    #[error("campaign state does not permit {operation}: {detail}")]
    InvalidState {
        /// The requested operation.
        operation: &'static str,
        /// The state detail.
        detail: String,
    },
    /// A proposal strategy returned an error.
    #[error("proposal strategy failed: {detail}")]
    Proposal {
        /// The strategy error detail.
        detail: String,
    },
    /// An adapter operation failed.
    #[error("{adapter} failed during {operation}: {source}")]
    Adapter {
        /// The adapter identity.
        adapter: String,
        /// The operation name.
        operation: &'static str,
        /// The adapter error.
        #[source]
        source: AdapterError,
    },
    /// An operation and the required candidate reconciliation both failed.
    #[error(
        "{operation} failed: {primary}; candidate reconciliation also failed: {reconciliation}"
    )]
    OperationAndReconciliationFailed {
        /// The operation that failed before reconciliation.
        operation: &'static str,
        /// The primary operation error.
        primary: Box<TuneError>,
        /// The reconciliation error.
        #[source]
        reconciliation: Box<TuneError>,
    },
    /// A gate or metric evaluator operation failed.
    #[error("{implementation} failed during {operation}: {source}")]
    Evaluator {
        /// The evaluator identity.
        implementation: String,
        /// The operation name.
        operation: &'static str,
        /// The evaluator error.
        #[source]
        source: EvaluatorError,
    },
    /// A simulator, scenario, or candidate receipt did not match the request.
    #[error("receipt validation failed during {operation}: {detail}")]
    ReceiptMismatch {
        /// The operation name.
        operation: &'static str,
        /// The mismatch detail.
        detail: String,
    },
    /// The initial candidate did not complete a safe training baseline.
    #[error("the training baseline is not safe: {detail}")]
    UnsafeBaseline {
        /// The failure detail.
        detail: String,
    },
    /// Another process owns the journal writer lock.
    #[error("another process owns the tuning journal at {path:?}")]
    JournalLocked {
        /// The journal root.
        path: PathBuf,
        /// The structured writer-lease conflict.
        #[source]
        source: Box<pilotage_durable_storage::StorageError>,
    },
    /// The live journal has an unresolved durable publication result.
    #[error("the live tuning journal is poisoned")]
    JournalPoisoned,
    /// The private durable store rejected a journal operation.
    #[error("durable journal storage failed: {source}")]
    Storage {
        /// The structured durable-storage failure.
        #[source]
        source: Box<pilotage_durable_storage::StorageError>,
    },
    /// Journal authorization and its temporary cleanup both failed.
    #[error(
        "journal authorization failed: {authorization}; temporary cleanup also failed: {cleanup}"
    )]
    AuthorizationAndCleanupFailed {
        /// The authorization failure.
        authorization: Box<TuneError>,
        /// The durable-storage cleanup failure.
        #[source]
        cleanup: Box<pilotage_durable_storage::StorageError>,
    },
    /// A journal belongs to a different tuning session.
    #[error("journal session does not match the requested session")]
    JournalSessionMismatch,
    /// A journal chain or state transition is not valid.
    #[error("journal is not valid: {detail}")]
    InvalidJournal {
        /// The validation detail.
        detail: String,
    },
    /// Stored bytes do not match their content digest.
    #[error("stored object {expected} has a different content digest")]
    DigestMismatch {
        /// The expected digest.
        expected: Digest,
    },
    /// A stored or pending document is too large.
    #[error("{document} has {size} bytes; limit is {limit}")]
    DocumentTooLarge {
        /// The document name or path.
        document: String,
        /// The document size.
        size: u64,
        /// The size limit.
        limit: u64,
    },
    /// A file operation failed.
    #[error("cannot {operation} {path:?}: {source}")]
    Io {
        /// The operation name.
        operation: &'static str,
        /// The affected path.
        path: PathBuf,
        /// The file error.
        #[source]
        source: std::io::Error,
    },
    /// A JSON document cannot be encoded.
    #[error("cannot encode {document}: {source}")]
    Encode {
        /// The document name.
        document: &'static str,
        /// The JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// A JSON document cannot be decoded.
    #[error("cannot decode {document} at {path:?}: {source}")]
    Decode {
        /// The document name.
        document: &'static str,
        /// The document path.
        path: PathBuf,
        /// The JSON error.
        #[source]
        source: serde_json::Error,
    },
}

impl TuneError {
    pub(crate) fn poisons_journal(&self) -> bool {
        match self {
            Self::JournalPoisoned
            | Self::DigestMismatch { .. }
            | Self::AuthorizationAndCleanupFailed { .. } => true,
            Self::Storage { source } | Self::JournalLocked { source, .. } => {
                source.poisons_authorization()
            }
            _ => false,
        }
    }
}

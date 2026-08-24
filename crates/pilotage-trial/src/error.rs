//! Errors for trial contract processing.

use thiserror::Error;

/// An error in the content of a trial document.
#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    /// A document uses an unsupported schema version.
    #[error("{document} schema version {actual} is not supported; expected {expected}")]
    UnsupportedSchemaVersion {
        /// The document name.
        document: &'static str,
        /// The supplied version.
        actual: u16,
        /// The supported version.
        expected: u16,
    },
    /// A required text value is empty.
    #[error("{field} must not be empty")]
    EmptyText {
        /// The field path.
        field: String,
    },
    /// A text value is too long.
    #[error("{field} has {size} bytes; the limit is {limit}")]
    TextTooLong {
        /// The field path.
        field: String,
        /// The supplied byte count.
        size: usize,
        /// The maximum byte count.
        limit: usize,
    },
    /// A required digest contains only zero bytes.
    #[error("{field} must not be a zero digest")]
    ZeroDigest {
        /// The field path.
        field: String,
    },
    /// A required list is empty.
    #[error("{field} must contain an item")]
    EmptyList {
        /// The field path.
        field: String,
    },
    /// A list has too many items.
    #[error("{field} has {count} items; the limit is {limit}")]
    TooManyItems {
        /// The field path.
        field: String,
        /// The supplied item count.
        count: usize,
        /// The maximum item count.
        limit: usize,
    },
    /// A list contains a duplicate item.
    #[error("{field} contains a duplicate item at index {index}")]
    DuplicateItem {
        /// The field path.
        field: String,
        /// The index of the second item.
        index: usize,
    },
    /// A number is not finite.
    #[error("{field} must be finite")]
    NonFinite {
        /// The field path.
        field: String,
    },
    /// A number is outside its permitted range.
    #[error("{field} value {actual} is outside {minimum} through {maximum}")]
    OutOfRange {
        /// The field path.
        field: String,
        /// The supplied value.
        actual: f64,
        /// The minimum value.
        minimum: f64,
        /// The maximum value.
        maximum: f64,
    },
    /// A duration is zero.
    #[error("{field} must be greater than zero")]
    ZeroDuration {
        /// The field path.
        field: String,
    },
    /// A relation between identity fields is invalid.
    #[error("identity mismatch for {field}")]
    IdentityMismatch {
        /// The field path.
        field: String,
    },
    /// A clock mapping is invalid.
    #[error("clock mapping {index} is invalid: {reason}")]
    InvalidClockMapping {
        /// The mapping index.
        index: usize,
        /// The cause of the error.
        reason: &'static str,
    },
    /// A stage source has no mapping for its clock epoch.
    #[error("{field} has no mapping for {clock} epoch {epoch}")]
    MissingClockMapping {
        /// The field path.
        field: String,
        /// The clock domain name.
        clock: String,
        /// The source clock epoch.
        epoch: u64,
    },
    /// A present stage value uses an unusable clock mapping.
    #[error("{field} uses an unusable mapping for {clock} epoch {epoch}")]
    UnusableClockMapping {
        /// The field path.
        field: String,
        /// The clock domain name.
        clock: String,
        /// The source clock epoch.
        epoch: u64,
    },
    /// A stage source time is outside the mapping validity interval.
    #[error("{field} time {time_ns} ns is outside the mapping for {clock} epoch {epoch}")]
    ClockTimeOutsideMapping {
        /// The field path.
        field: String,
        /// The clock domain name.
        clock: String,
        /// The source clock epoch.
        epoch: u64,
        /// The source time.
        time_ns: u64,
    },
    /// A clock observation is not internally consistent.
    #[error("{field} has an invalid clock observation: {reason}")]
    InvalidClockObservation {
        /// The field path.
        field: String,
        /// The cause of the error.
        reason: &'static str,
    },
    /// A causal stage stamp is not internally consistent.
    #[error("{field} has an invalid stage stamp: {reason}")]
    InvalidStageStamp {
        /// The field path.
        field: String,
        /// The cause of the error.
        reason: &'static str,
    },
    /// A derived control event references an event that is not in the trace.
    #[error("{field} references unknown {stage} event {clock} epoch {epoch} sequence {sequence}")]
    UnknownControlPredecessor {
        /// The derived stage field path.
        field: String,
        /// The predecessor stage name.
        stage: String,
        /// The predecessor clock domain name.
        clock: String,
        /// The predecessor clock epoch.
        epoch: u64,
        /// The predecessor event sequence number.
        sequence: u64,
    },
    /// A mapped control event occurs definitely before its predecessor.
    #[error(
        "{field} mapped latest time {current_latest_ns} ns is before predecessor earliest time {predecessor_earliest_ns} ns"
    )]
    CausalMappedSourceOrder {
        /// The derived stage field path.
        field: String,
        /// The predecessor earliest mapped time.
        predecessor_earliest_ns: u64,
        /// The derived event latest mapped time.
        current_latest_ns: u64,
    },
    /// An attitude quaternion is not a unit quaternion.
    #[error("{field} norm {norm} differs from one by more than {tolerance}")]
    InvalidQuaternionNorm {
        /// The field path.
        field: String,
        /// The supplied quaternion norm.
        norm: f64,
        /// The permitted difference from one.
        tolerance: f64,
    },
    /// A phase needs a capability that the backend does not supply.
    #[error("phase {phase} needs unsupported capability {capability}")]
    UnsupportedCapability {
        /// The phase identifier.
        phase: String,
        /// The capability name.
        capability: String,
    },
    /// A sample uses a phase index that is outside the scenario.
    #[error("sample phase index {index} is outside {phase_count} phases")]
    PhaseOutOfRange {
        /// The supplied phase index.
        index: u16,
        /// The scenario phase count.
        phase_count: usize,
    },
    /// Two adjacent samples have different run digests.
    #[error("adjacent samples have different run digests")]
    MixedRun,
    /// A sample sequence number is not after the prior number.
    #[error("sample sequence {current} does not follow {previous}")]
    SequenceOrder {
        /// The prior sequence number.
        previous: u64,
        /// The current sequence number.
        current: u64,
    },
    /// A sample gap does not agree with its gap count.
    #[error("sample sequence {actual} does not match expected sequence {expected}")]
    SequenceGap {
        /// The sequence number that includes the declared gap.
        expected: u64,
        /// The supplied sequence number.
        actual: u64,
    },
    /// A sample returns to an earlier phase.
    #[error("sample phase {current} is before prior phase {previous}")]
    PhaseOrder {
        /// The prior phase index.
        previous: u16,
        /// The current phase index.
        current: u16,
    },
    /// A clock moves back without a discontinuity record.
    #[error("{clock} clock moved from {previous_ns} ns to {current_ns} ns")]
    ClockRegression {
        /// The clock domain name.
        clock: String,
        /// The prior time.
        previous_ns: u64,
        /// The current time.
        current_ns: u64,
    },
}

/// An error during trial JSON processing.
#[derive(Debug, Error)]
pub enum CodecError {
    /// A JSON document exceeds its fixed size limit.
    #[error("{document} has {size} bytes; the limit is {limit}")]
    DocumentTooLarge {
        /// The document name.
        document: &'static str,
        /// The supplied byte count.
        size: usize,
        /// The maximum byte count.
        limit: usize,
    },
    /// JSON decoding failed.
    #[error("cannot decode {document} JSON")]
    Decode {
        /// The document name.
        document: &'static str,
        /// The JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// JSON encoding failed.
    #[error("cannot encode {document} JSON")]
    Encode {
        /// The document name.
        document: &'static str,
        /// The JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// Content validation failed.
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

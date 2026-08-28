//! Mission document errors.

use thiserror::Error;

use crate::{Digest, ExecutionTarget, MissionCapability};

/// An error in mission document content.
#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    /// The schema version is not supported.
    #[error("mission schema version {actual} is not supported; expected {expected}")]
    UnsupportedSchemaVersion {
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
    /// A phase identifier occurs more than once.
    #[error("mission.phases contains repeated phase id {phase_id} at index {index}")]
    RepeatedPhaseId {
        /// The repeated phase identifier.
        phase_id: String,
        /// The index of the second occurrence.
        index: usize,
    },
    /// A capability occurs more than once in a phase declaration.
    #[error("phase {phase_id} repeats capability {capability:?}")]
    RepeatedCapability {
        /// The phase identifier.
        phase_id: String,
        /// The repeated capability.
        capability: MissionCapability,
    },
    /// A phase has no simulator-time deadline.
    #[error("phase {phase_id} must have a simulator-time deadline")]
    MissingDeadline {
        /// The phase identifier.
        phase_id: String,
    },
    /// A duration is zero.
    #[error("{field} must be greater than zero")]
    ZeroDuration {
        /// The field path.
        field: String,
    },
    /// A phase uses a capability that it does not declare.
    #[error("phase {phase_id} does not declare capability {capability:?}")]
    UndeclaredCapability {
        /// The phase identifier.
        phase_id: String,
        /// The required capability.
        capability: MissionCapability,
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
    /// A stimulus does not name one valid physical command.
    #[error("{field} has an invalid stimulus: {source}")]
    InvalidStimulus {
        /// The field path.
        field: String,
        /// The physical contract failure.
        #[source]
        source: crate::StimulusError,
    },
    /// A flight plan uses different navigation data from the mission.
    #[error("phase {phase_id} flight plan {plan_id} has different navigation-data identity")]
    NavigationDataMismatch {
        /// The phase identifier.
        phase_id: String,
        /// The flight-plan identifier.
        plan_id: String,
    },
    /// The document target does not match the admission target.
    #[error("mission target {document_target:?} does not match host target {host_target:?}")]
    ExecutionTargetMismatch {
        /// The target in the document.
        document_target: ExecutionTarget,
        /// The host target.
        host_target: ExecutionTarget,
    },
    /// A real-vehicle target contains a simulator-only action.
    #[error("phase {phase_id} action {action} uses the simulator-only transport lane")]
    SimulatorOnlyAction {
        /// The phase identifier.
        phase_id: String,
        /// The stable action name.
        action: &'static str,
    },
    /// The declared content digest does not match the document content.
    #[error("mission content digest {declared} does not match calculated digest {calculated}")]
    ContentDigestMismatch {
        /// The declared digest.
        declared: Digest,
        /// The calculated digest.
        calculated: Digest,
    },
}

/// An error in mission document encoding or decoding.
#[derive(Debug, Error)]
pub enum CodecError {
    /// JSON decoding failed.
    #[error("cannot decode mission document: {source}")]
    Decode {
        /// The JSON decoder error.
        #[source]
        source: serde_json::Error,
    },
    /// JSON encoding failed.
    #[error("cannot encode mission document: {source}")]
    Encode {
        /// The JSON encoder error.
        #[source]
        source: serde_json::Error,
    },
    /// The encoded document is too large.
    #[error("mission document has {size} bytes; the limit is {limit}")]
    DocumentTooLarge {
        /// The supplied byte count.
        size: usize,
        /// The maximum byte count.
        limit: usize,
    },
    /// The document content is invalid.
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

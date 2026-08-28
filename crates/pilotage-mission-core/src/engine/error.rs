//! Mission engine admission and input errors.

use thiserror::Error;

use crate::{ActionId, CodecError, Digest, ValidationError};

/// An error that prevents a mission engine from starting.
#[derive(Debug, Error)]
pub enum EngineStartError {
    /// The mission document is not valid for the host.
    #[error("the mission document is not valid for the host: {source}")]
    Admission {
        /// The document validation error.
        #[source]
        source: ValidationError,
    },
    /// The mission content digest is not valid.
    #[error("the mission document digest is not valid: {source}")]
    DocumentIdentity {
        /// The document codec error.
        #[source]
        source: CodecError,
    },
    /// The wall deadline belongs to a different mission.
    #[error("wall deadline mission digest {actual} does not match {expected}")]
    WallDeadlineIdentity {
        /// The mission document digest.
        expected: Digest,
        /// The deadline mission digest.
        actual: Digest,
    },
    /// The wall deadline is not later than the admission clock.
    #[error("wall deadline {deadline_ns} is not later than start wall time {wall_time_ns}")]
    WallDeadlineExpired {
        /// The wall clock at admission.
        wall_time_ns: u64,
        /// The supplied absolute deadline.
        deadline_ns: u64,
    },
}

/// An invalid input to one engine tick.
#[derive(Debug, Error, PartialEq)]
pub enum EngineInputError {
    /// A caller tick followed a terminal result.
    #[error("the mission engine is terminal")]
    Terminal {},
    /// The simulator clock moved backwards.
    #[error("simulator clock moved backwards from {previous_ns} to {current_ns}")]
    SimulatorClockRegressed {
        /// The last accepted simulator clock.
        previous_ns: u64,
        /// The supplied simulator clock.
        current_ns: u64,
    },
    /// The wall clock moved backwards.
    #[error("wall clock moved backwards from {previous_ns} to {current_ns}")]
    WallClockRegressed {
        /// The last accepted wall clock.
        previous_ns: u64,
        /// The supplied wall clock.
        current_ns: u64,
    },
    /// A tick supplied more than one receipt for the one outstanding directive.
    #[error("a tick supplied {count} receipts; at most one receipt is permitted")]
    TooManyReceipts {
        /// The supplied receipt count.
        count: usize,
    },
    /// A receipt does not identify the outstanding directive.
    #[error("receipt action id {received:?} does not match outstanding id {outstanding:?}")]
    StaleReceipt {
        /// The received identifier.
        received: ActionId,
        /// The outstanding identifier, when one exists.
        outstanding: Option<ActionId>,
    },
    /// An observation contains a non-finite number.
    #[error("{field} must be finite")]
    NonFiniteObservation {
        /// The invalid observation field.
        field: String,
    },
    /// An observation repeats one exact signal selector.
    #[error("observation signal at index {index} repeats an earlier selector")]
    RepeatedSignal {
        /// The second signal index.
        index: usize,
    },
}

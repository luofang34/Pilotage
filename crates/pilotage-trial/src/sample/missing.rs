//! Explicit missing signal records.

use serde::{Deserialize, Serialize};

use crate::{MAX_TEXT_BYTES, ValidationError, validation::optional_text};

/// The cause of a missing signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingReason {
    /// The source did not publish the signal.
    NotPublished,
    /// The signal does not apply to this backend or phase.
    NotApplicable,
    /// A component rejected the value.
    Rejected,
    /// The most recent value is too old.
    Stale,
    /// The source marked the value as invalid.
    Invalid,
    /// A sequence gap removed the value.
    SequenceGap,
    /// Recorder lag removed the value.
    RecorderLag,
    /// A clock discontinuity prevents alignment.
    ClockDiscontinuity,
}

/// A record that explains one missing signal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissingSignal {
    /// The missing signal reason.
    pub reason: MissingReason,
    /// Additional bounded information about the reason.
    pub detail: Option<String>,
}

impl MissingSignal {
    pub(crate) fn validate(&self, field: &str) -> Result<(), ValidationError> {
        optional_text(
            &format!("{field}.detail"),
            self.detail.as_deref(),
            MAX_TEXT_BYTES,
        )
    }
}

/// A present value or an explicit missing signal record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Observed<T> {
    /// The signal has a value.
    Present {
        /// The signal value.
        value: T,
    },
    /// The signal has no value.
    Missing {
        /// The missing signal record.
        missing: MissingSignal,
    },
}

impl<T> Observed<T> {
    /// Creates a present signal value.
    #[must_use]
    pub const fn present(value: T) -> Self {
        Self::Present { value }
    }

    /// Creates a missing signal record.
    #[must_use]
    pub const fn missing(reason: MissingReason, detail: Option<String>) -> Self {
        Self::Missing {
            missing: MissingSignal { reason, detail },
        }
    }

    /// Gets the present value.
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        match self {
            Self::Present { value } => Some(value),
            Self::Missing { .. } => None,
        }
    }

    /// Gets the missing signal record.
    #[must_use]
    pub const fn missing_signal(&self) -> Option<&MissingSignal> {
        match self {
            Self::Present { .. } => None,
            Self::Missing { missing } => Some(missing),
        }
    }

    pub(crate) fn validate_with<F>(&self, field: &str, validate: F) -> Result<(), ValidationError>
    where
        F: FnOnce(&T, &str) -> Result<(), ValidationError>,
    {
        match self {
            Self::Present { value } => validate(value, field),
            Self::Missing { missing } => missing.validate(field),
        }
    }
}

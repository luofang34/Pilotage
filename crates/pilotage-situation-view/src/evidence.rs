//! Evidence states that do not use sentinel values.

use serde::{Deserialize, Serialize};

/// Reason that a requested value or item is not present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingDataReasonV1 {
    /// The producer has not accepted evidence for the item.
    NotObserved,
    /// A source reported that the item is not available.
    SourceReportedUnavailable,
    /// The item passed its freshness or validity limit.
    Expired,
    /// The item failed an acceptance rule.
    Rejected,
    /// The item has no meaning for the selected subject.
    NotApplicable,
    /// The producer cannot represent the item.
    Unsupported,
    /// A declared resource limit prevented retention.
    ResourceLimit,
    /// The selected domain is not installed or is not available.
    DomainUnavailable,
    /// The selected knowledge state is not in the retained data.
    KnowledgeStateUnavailable,
    /// Valid time evidence is not available.
    InvalidTimeEvidence,
    /// The domain schema defines a more specific reason.
    DomainSpecific {
        /// Stable reason code from the domain schema.
        code: String,
    },
}

/// A present value or an explicit reason for a missing value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum EvidenceV1<T> {
    /// The evidence contains a value.
    Present {
        /// The evidence value.
        value: T,
    },
    /// The evidence does not contain a value.
    Missing {
        /// Reason that the value is not present.
        reason: MissingDataReasonV1,
    },
}

impl<T> EvidenceV1<T> {
    /// Returns the present value.
    #[must_use]
    pub const fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Present { value } => Some(value),
            Self::Missing { .. } => None,
        }
    }
}

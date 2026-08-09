//! Civil and monotonic time types for the contract.

use serde::{Deserialize, Serialize};

use crate::EvidenceV1;

/// A UTC instant with nanosecond resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtcInstantV1 {
    /// Whole seconds from the Unix epoch.
    pub unix_seconds: i64,
    /// Nanoseconds after `unix_seconds`.
    pub subsecond_nanoseconds: u32,
}

impl UtcInstantV1 {
    /// Returns the instant as a signed nanosecond count.
    #[must_use]
    pub fn unix_nanoseconds(self) -> Option<i128> {
        if self.subsecond_nanoseconds >= 1_000_000_000 {
            return None;
        }
        Some(
            i128::from(self.unix_seconds)
                .saturating_mul(1_000_000_000)
                .saturating_add(i128::from(self.subsecond_nanoseconds)),
        )
    }
}

/// An inclusive UTC interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtcIntervalV1 {
    /// First valid UTC instant.
    pub start: UtcInstantV1,
    /// Last valid UTC instant.
    pub end: UtcInstantV1,
}

/// A point on one identified monotonic clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonotonicStampV1 {
    /// Opaque identity for one continuous monotonic clock.
    pub clock_id: String,
    /// Nanoseconds from the local clock origin.
    pub nanoseconds: u64,
}

/// An inclusive interval on one monotonic clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonotonicIntervalV1 {
    /// First stamp for which the mapping is valid.
    pub start_nanoseconds: u64,
    /// Last stamp for which the mapping is valid.
    pub end_nanoseconds: u64,
}

impl MonotonicIntervalV1 {
    /// Returns true when `stamp` is in the inclusive interval.
    #[must_use]
    pub const fn contains(&self, stamp: u64) -> bool {
        self.start_nanoseconds <= stamp && stamp <= self.end_nanoseconds
    }
}

/// A bounded mapping from a source clock to a target clock.
///
/// The mapping is `target = source + offset_nanoseconds`. The uncertainty is
/// a symmetric error bound. The mapping applies only in `valid_source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockCorrespondenceV1 {
    /// Clock that supplied the source stamp.
    pub source_clock_id: String,
    /// Clock to which the source stamp maps.
    pub target_clock_id: String,
    /// Signed offset from the source clock to the target clock.
    pub offset_nanoseconds: i64,
    /// Symmetric mapping error bound.
    pub uncertainty_nanoseconds: u64,
    /// Source-clock interval in which the mapping is valid.
    pub valid_source: MonotonicIntervalV1,
}

/// Quality of a source observation or product time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeQualityV1 {
    /// A trusted source supplied the time.
    Trusted,
    /// A documented mapping or estimate supplied the time.
    Estimated,
    /// The source time is not valid for age calculation.
    Untrusted,
}

/// Reason that an age cannot be calculated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeUnknownReasonV1 {
    /// The ingress stamp is not present.
    MissingIngressTime,
    /// The source time is not present.
    MissingSourceTime,
    /// The source time quality is not present.
    MissingTimeQuality,
    /// The source time quality does not permit an age calculation.
    UntrustedSourceTime,
    /// No clock correspondence maps the ingress stamp to the host clock.
    MissingClockCorrespondence,
    /// The ingress stamp is outside the mapping valid interval.
    ClockCorrespondenceOutOfRange,
    /// More than one valid mapping applies to the ingress stamp.
    AmbiguousClockCorrespondence,
    /// A clock mapping cannot produce a monotonic stamp.
    InvalidClockCorrespondence,
    /// The ingress stamp is after the evaluation stamp.
    IngressAfterEvaluation,
    /// The source time is after the evaluation time.
    SourceTimeAfterEvaluation,
    /// A UTC value is outside the contract representation.
    InvalidUtcTime,
    /// The age is larger than the contract representation.
    AgeOverflow,
}

/// A calculated age or a reason that the age is unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum AgeV1 {
    /// The age value is known.
    Known {
        /// Age at the evaluation point.
        nanoseconds: u64,
        /// Symmetric error bound for the age.
        uncertainty_nanoseconds: EvidenceV1<u64>,
    },
    /// The required time evidence is not valid.
    Unknown {
        /// Reason that the age is unknown.
        reason: AgeUnknownReasonV1,
    },
}

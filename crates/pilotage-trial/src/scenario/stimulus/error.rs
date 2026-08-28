//! Errors for the physical stimulus contract.

use thiserror::Error;

/// An error in the physical meaning of one stimulus.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum StimulusError {
    /// An envelope endpoint or neutral value is not finite.
    #[error("the {name} value must be finite")]
    NonFiniteValue {
        /// The envelope value name.
        name: &'static str,
    },
    /// The negative endpoint is above the positive endpoint.
    #[error("the negative endpoint {negative} is above the positive endpoint {positive}")]
    ReversedEndpoints {
        /// The negative endpoint.
        negative: f64,
        /// The positive endpoint.
        positive: f64,
    },
    /// The two endpoints describe no physical span.
    #[error("both endpoints are {value}, which is a zero physical span")]
    ZeroSpan {
        /// The value that both endpoints have.
        value: f64,
    },
    /// The neutral value is not between the two endpoints.
    #[error("the neutral value {neutral} must be between {negative} and {positive}")]
    NeutralOutsideEndpoints {
        /// The neutral value.
        neutral: f64,
        /// The negative endpoint.
        negative: f64,
        /// The positive endpoint.
        positive: f64,
    },
    /// The physical unit does not belong to the control family and channel.
    #[error("family {family} channel {channel} uses unit {expected}, not {actual}")]
    UnitMismatch {
        /// The control family name.
        family: &'static str,
        /// The control channel name.
        channel: &'static str,
        /// The unit that the combination requires.
        expected: &'static str,
        /// The supplied unit.
        actual: &'static str,
    },
    /// The reference rule does not belong to the control family and channel.
    #[error("family {family} channel {channel} uses reference {expected}, not {actual}")]
    ReferenceMismatch {
        /// The control family name.
        family: &'static str,
        /// The control channel name.
        channel: &'static str,
        /// The reference rule that the combination requires.
        expected: &'static str,
        /// The supplied reference rule.
        actual: &'static str,
    },
    /// The mapping rule does not belong to the control family.
    #[error("family {family} uses mapping {expected}, not {actual}")]
    MappingMismatch {
        /// The control family name.
        family: &'static str,
        /// The mapping rule that the family requires.
        expected: &'static str,
        /// The supplied mapping rule.
        actual: &'static str,
    },
    /// The mapping rule resolves no exact value at authoring time.
    #[error("mapping {mapping} needs a candidate feel profile to resolve one exact value")]
    InexactMapping {
        /// The mapping rule name.
        mapping: &'static str,
    },
    /// A normalized value is outside minus one through plus one.
    #[error("the normalized value {value} is outside minus one through plus one")]
    NormalizedOutOfRange {
        /// The supplied normalized value.
        value: f64,
    },
}

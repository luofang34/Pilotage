//! Runtime scalar selection errors.

use thiserror::Error;

/// An error from selecting a scalar from one trial value.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum SignalSelectionError {
    /// The tagged control value has a different variant.
    #[error("control selector {selector} requires {expected}; found {actual}")]
    ControlValueVariantMismatch {
        /// The scalar selector name.
        selector: &'static str,
        /// The required control value variant.
        expected: &'static str,
        /// The supplied control value variant.
        actual: &'static str,
    },
    /// The control value uses a different reference frame.
    #[error("control selector {selector} requires frame {expected}; found {actual}")]
    ReferenceFrameMismatch {
        /// The scalar selector name.
        selector: &'static str,
        /// The required reference frame.
        expected: &'static str,
        /// The supplied reference frame.
        actual: &'static str,
    },
    /// The scalar channel is not in the control value.
    #[error("scalar channel {index} is not in a control value with {count} channels")]
    ScalarChannelUnavailable {
        /// The selected channel index.
        index: u16,
        /// The available channel count.
        count: usize,
    },
}

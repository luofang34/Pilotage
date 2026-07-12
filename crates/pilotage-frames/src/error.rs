//! Typed failures for frame composition and decoding.

use crate::frame::FrameId;
use crate::time::Epoch;

/// Why a frame operation was refused. Every variant is fail-closed:
/// nothing composes, inverts, or decodes through a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The wire carried a frame id this build cannot place.
    UnknownFrame {
        /// The unrecognized code.
        code: u8,
    },
    /// Two operands disagree about the frame at their junction.
    FrameMismatch {
        /// The frame the operation required.
        expected: FrameId,
        /// The frame it found.
        found: FrameId,
    },
    /// Two operands share clock and scale but not the instant; a
    /// rotating-frame transform applied at the wrong instant silently
    /// relabels geometry, so exact identity is required.
    EpochMismatch {
        /// The epoch the operation required.
        expected: Epoch,
        /// The epoch it found.
        found: Epoch,
    },
    /// Two operands' epochs come from different clocks.
    ClockMismatch,
    /// Two operands' epochs use different time scales.
    TimeScaleMismatch,
    /// The offered rotation is not one (zero, gross, or non-finite
    /// norm), or a translation component is non-finite.
    InvalidTransform,
}

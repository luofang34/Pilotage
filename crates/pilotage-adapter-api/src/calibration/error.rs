//! Typed, fail-closed calibration errors.

use super::identity::ValidityStatus;

/// Why a calibration cannot be used. Every variant carries the context its
/// message needs; none has a benign fallback — a calibration that fails any
/// check disables conformal output rather than degrading to a guess.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CalibrationError {
    /// The recomputed content hash did not match the recorded one: the
    /// artifact was altered without re-recording its hash, or is corrupt.
    #[error("calibration content hash mismatch")]
    ContentHashMismatch {
        /// The recorded hash the artifact claims.
        expected: [u8; 32],
        /// The hash recomputed over the canonical bytes.
        computed: [u8; 32],
    },
    /// The calibration's status is not `Valid`.
    #[error("calibration is not valid for use: status {status:?}")]
    NotValid {
        /// The status that blocked use.
        status: ValidityStatus,
    },
    /// The evaluation time is outside the effective window.
    #[error(
        "calibration not effective at {now_unix_ns} ns \
         (window [{start_unix_ns}, {end_unix_ns}))"
    )]
    Expired {
        /// The evaluation time, Unix nanoseconds.
        now_unix_ns: u64,
        /// Window start, Unix nanoseconds.
        start_unix_ns: u64,
        /// Window end, Unix nanoseconds.
        end_unix_ns: u64,
    },
    /// The calibration describes a different camera than the frame's.
    #[error("calibration is for camera {expected}, frame is from camera {actual}")]
    WrongCamera {
        /// Camera id the calibration describes.
        expected: u32,
        /// Camera id the frame came from.
        actual: u32,
    },
}

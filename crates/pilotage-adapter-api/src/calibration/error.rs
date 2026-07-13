//! Typed, fail-closed calibration errors.

use super::identity::ValidityStatus;

/// Why a calibration cannot be used. Every variant carries the context its
/// message needs; none has a benign fallback — a calibration that fails any
/// check disables conformal output rather than degrading to a guess.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
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
    /// A field that must be finite is NaN or infinite. Named so the offending
    /// quantity is identifiable without repairing it.
    #[error("calibration field {field} is not finite")]
    NonFinite {
        /// The non-finite field.
        field: &'static str,
    },
    /// The viewport has a zero (or otherwise invalid) dimension.
    #[error("calibration viewport {width_px}x{height_px} is degenerate")]
    InvalidViewport {
        /// Viewport width, pixels.
        width_px: u32,
        /// Viewport height, pixels.
        height_px: u32,
    },
    /// A focal length is not strictly positive.
    #[error("calibration focal lengths ({focal_x_px}, {focal_y_px} px) are not positive")]
    NonPositiveFocal {
        /// Focal length x, pixels.
        focal_x_px: f64,
        /// Focal length y, pixels.
        focal_y_px: f64,
    },
    /// The principal point lies outside the viewport.
    #[error(
        "calibration principal point ({principal_x_px}, {principal_y_px}) is outside \
         the {width_px}x{height_px} viewport"
    )]
    PrincipalPointOutOfBounds {
        /// Principal point x, pixels.
        principal_x_px: f64,
        /// Principal point y, pixels.
        principal_y_px: f64,
        /// Viewport width, pixels.
        width_px: u32,
        /// Viewport height, pixels.
        height_px: u32,
    },
    /// The extrinsics name frames other than body → installation.
    #[error("calibration extrinsics frames {from_code}->{to_code} are not body->installation")]
    FrameMismatch {
        /// The declared source frame code.
        from_code: u8,
        /// The declared target frame code.
        to_code: u8,
    },
    /// The extrinsic rotation quaternion is not unit-norm.
    #[error("calibration extrinsic quaternion norm {norm} is not 1")]
    NonUnitQuaternion {
        /// The quaternion's norm.
        norm: f64,
    },
    /// The boresight direction is not a unit vector.
    #[error("calibration boresight norm {norm} is not 1")]
    NonUnitBoresight {
        /// The boresight's norm.
        norm: f64,
    },
    /// The effective window is empty or inverted (`end <= start`).
    #[error("calibration effective window [{start_unix_ns}, {end_unix_ns}) is invalid")]
    InvalidEffectivePeriod {
        /// Window start, Unix nanoseconds.
        start_unix_ns: u64,
        /// Window end, Unix nanoseconds.
        end_unix_ns: u64,
    },
    /// The residuals are negative, non-finite, or the RMS exceeds the maximum.
    #[error("calibration residuals (rms={rms_px}, max={max_px} px) are invalid")]
    InvalidResiduals {
        /// RMS residual, pixels.
        rms_px: f64,
        /// Max residual, pixels.
        max_px: f64,
    },
    /// A declared alignment allowance is not strictly positive. Named so the
    /// zeroed allowance is identifiable.
    #[error("calibration alignment allowance {which} is not strictly positive")]
    NonPositiveAllowance {
        /// Which allowance was non-positive.
        which: &'static str,
    },
    /// The intrinsic residual budget does not cover the measured recovery
    /// residual — the calibration would understate its own fit error.
    #[error(
        "calibration intrinsic residual budget {intrinsic_residual_px} px does not cover \
         the measured residual {measured_max_px} px"
    )]
    IntrinsicResidualBelowMeasured {
        /// The declared intrinsic residual budget, pixels.
        intrinsic_residual_px: f64,
        /// The measured maximum recovery residual, pixels.
        measured_max_px: f64,
    },
}

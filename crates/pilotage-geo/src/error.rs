//! Typed, fail-closed failure model for the geospatial contract.
//!
//! Every variant is a refusal, not a repair: a construction or decode that
//! cannot be trusted returns an error instead of guessing a datum, a unit, a
//! frame, or a clock.

/// Why constructing or validating a geospatial value failed.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum GeoError {
    /// A field that must be finite is NaN or infinite.
    #[error("geospatial field {field} is not finite")]
    NonFinite {
        /// The offending field.
        field: &'static str,
    },
    /// Latitude is outside `[-90, 90]` degrees.
    #[error("latitude {lat_deg} deg is outside [-90, 90]")]
    LatitudeOutOfRange {
        /// The offending latitude, degrees.
        lat_deg: f64,
    },
    /// Longitude is outside `[-180, 180]` degrees (before normalization).
    #[error("longitude {lon_deg} deg is outside [-180, 180]")]
    LongitudeOutOfRange {
        /// The offending longitude, degrees.
        lon_deg: f64,
    },
    /// The horizontal datum is unknown to this build; the position has no
    /// interpretable frame and is refused rather than guessed.
    #[error("horizontal datum is unknown; position has no interpretable frame")]
    UnknownHorizontalDatum,
    /// A realization-bearing horizontal datum (NAD83, ITRF) was supplied
    /// without a declared realization/reference-epoch identity, so the frame
    /// is ambiguous by centimeters-to-meters and is refused.
    #[error("horizontal datum realization/reference epoch was not declared")]
    UndeclaredDatumRealization,
    /// The vertical datum is unknown to this build; the height has no
    /// interpretable reference and is refused rather than guessed.
    #[error("vertical datum is unknown; height has no interpretable reference")]
    UnknownVerticalDatum,
    /// A geometric-MSL height was supplied without a declared geoid model, so
    /// the ellipsoid↔MSL separation is undefined.
    #[error("geometric-MSL height requires a declared geoid model")]
    UndeclaredGeoidModel,
    /// An AGL height was supplied without a declared terrain/ground reference,
    /// so the ground it is measured above is unidentified.
    #[error("AGL height requires a declared terrain/ground reference")]
    UndeclaredTerrainReference,
    /// A barometric-indicated height was supplied without a declared applied
    /// altimeter-setting identity, so its datum surface is unidentified.
    #[error("barometric-indicated height requires a declared applied-setting identity")]
    UndeclaredBaroSetting,
    /// A local-relative height was supplied without a declared local origin.
    #[error("local-relative height requires a declared local origin")]
    UndeclaredLocalOrigin,
    /// A tile size was non-positive or non-finite, so no tile index is defined.
    #[error("tile size {tile_deg} deg is not a positive finite value")]
    InvalidTileSize {
        /// The offending tile size, degrees.
        tile_deg: f64,
    },
    /// A viewport has a zero (or otherwise degenerate) dimension.
    #[error("viewport {width_px}x{height_px} is degenerate")]
    InvalidViewport {
        /// Viewport width, pixels.
        width_px: u32,
        /// Viewport height, pixels.
        height_px: u32,
    },
    /// A focal length is not strictly positive.
    #[error("focal lengths ({focal_x_px}, {focal_y_px} px) are not positive")]
    NonPositiveFocal {
        /// Focal length x, pixels.
        focal_x_px: f64,
        /// Focal length y, pixels.
        focal_y_px: f64,
    },
    /// The near/far clip policy is invalid (`near <= 0` or `far <= near`).
    #[error("near/far policy (near={near_m}, far={far_m} m) is invalid")]
    InvalidNearFar {
        /// Near clip distance, meters.
        near_m: f64,
        /// Far clip distance, meters.
        far_m: f64,
    },
    /// The accepted-calibration reference is incomplete: a zero id, an
    /// all-zero content hash, or a non-positive/non-finite alignment bound.
    /// The view must reference exactly one validated calibration artifact.
    #[error("calibration reference is incomplete or unbounded")]
    IncompleteCalibrationReference,
    /// An orthographic projection was declared without positive, finite metric
    /// extents; a focal-derived field of view is not an orthographic invariant.
    #[error("orthographic extents ({extent_x_m}, {extent_y_m} m) are not positive finite")]
    InvalidOrthographicExtent {
        /// Metric extent across the viewport width, meters.
        extent_x_m: f64,
        /// Metric extent across the viewport height, meters.
        extent_y_m: f64,
    },
    /// The camera attitude is not a unit rotation.
    #[error("camera attitude is not a unit rotation")]
    CameraAttitudeNotARotation,
    /// The camera pose does not map Body → Installation.
    #[error("camera pose must map Body -> Installation")]
    WrongCameraPoseFrames,
    /// The aircraft attitude quaternion is not a unit rotation.
    #[error("aircraft attitude is not a unit rotation")]
    AttitudeNotARotation,
}

/// Why an age could not be computed between two epochs. There is no
/// silently-inferred age: a future sample or an incompatible clock/scale is a
/// typed refusal, never a saturated zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AgeError {
    /// The two epochs are on different clock domains, so their difference is
    /// not a physical duration.
    #[error("age spans two clock domains")]
    ClockMismatch,
    /// The two epochs use different time scales, so their difference is not a
    /// physical duration.
    #[error("age spans two time scales")]
    ScaleMismatch,
    /// The sample was acquired after the reference time (a future sample); its
    /// age is undefined and it must not be treated as fresh.
    #[error("sample acquired {acquired_nanos} ns is after reference {now_nanos} ns")]
    FutureSample {
        /// The sample acquisition time, nanoseconds.
        acquired_nanos: u64,
        /// The reference (now) time, nanoseconds.
        now_nanos: u64,
    },
}

/// Why decoding a wire ABI block failed. Decode fails closed on any value this
/// build cannot interpret, never masquerading as a benign default.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum AbiError {
    /// The buffer length does not equal the fixed block length. A fixed-size
    /// block must match exactly: trailing bytes are as suspect as truncation.
    #[error("ABI block length wrong: need exactly {needed} bytes, got {got}")]
    WrongLength {
        /// Bytes the block requires.
        needed: usize,
        /// Bytes supplied.
        got: usize,
    },
    /// The leading version field is one this decoder does not read.
    #[error("ABI version {found} is not supported")]
    BadVersion {
        /// The version found.
        found: u32,
    },
    /// An enumerated field carried a value outside the known set.
    #[error("ABI field {field} carried unknown value {value}")]
    UnknownEnum {
        /// The field that was unknown.
        field: &'static str,
        /// The unrecognized wire value.
        value: u8,
    },
    /// A decoded floating-point field that must be finite is NaN or infinite.
    #[error("ABI field {field} decoded a non-finite value")]
    NonFinite {
        /// The offending field.
        field: &'static str,
    },
    /// The decoded values violate a semantic invariant (e.g. an MSL height with
    /// no declared geoid, a non-unit attitude), so the block is refused rather
    /// than trusted. Carries the geospatial reason that rejected it.
    #[error("ABI block is semantically malformed in {field}: {reason}")]
    Malformed {
        /// The block region that was malformed.
        field: &'static str,
        /// The geospatial validation reason.
        reason: GeoError,
    },
}

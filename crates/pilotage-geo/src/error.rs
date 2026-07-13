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
    /// The vertical datum is unknown to this build; the height has no
    /// interpretable reference and is refused rather than guessed.
    #[error("vertical datum is unknown; height has no interpretable reference")]
    UnknownVerticalDatum,
    /// A geometric-MSL height was supplied without a declared geoid model, so
    /// the ellipsoid↔MSL separation is undefined.
    #[error("geometric-MSL height requires a declared geoid model")]
    UndeclaredGeoidModel,
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
}

/// Why decoding a wire ABI block failed. Decode fails closed on any value this
/// build cannot interpret, never masquerading as a benign default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AbiError {
    /// The buffer is shorter than the fixed block length.
    #[error("ABI block truncated: need {needed} bytes, got {got}")]
    Truncated {
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
    /// no declared geoid), so the block is refused rather than trusted.
    #[error("ABI block is semantically malformed: {field}")]
    Malformed {
        /// What made the block malformed.
        field: &'static str,
    },
}

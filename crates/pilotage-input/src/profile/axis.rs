//! Axis calibration and configuration schema (ADR-0007 schema v1).

use serde::{Deserialize, Serialize};

/// The calibrated raw-unit range for one axis: the value observed at rest
/// (`center`), and the extremes observed at full deflection (`min`/`max`).
///
/// Ranges need not be symmetric around `center`; the normalization pipeline
/// (`crate::normalize`) scales each side of center independently.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisCalibration {
    /// Raw value at minimum (most negative) deflection.
    pub min: f32,
    /// Raw value at rest / neutral position.
    pub center: f32,
    /// Raw value at maximum (most positive) deflection.
    pub max: f32,
}

/// Configuration for a single physical axis, as declared in a device
/// profile's `axes` array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisConfig {
    /// Index of this axis in `RawDeviceSample::axes`.
    pub source_index: usize,
    /// Well-known logical axis name (resolved via `crate::logical`).
    pub logical: String,
    /// Whether the normalized output sign is flipped after the response
    /// curve is applied.
    pub invert: bool,
    /// Half-width of the dead zone around center, in normalized `[-1, 1]`
    /// units, before the response curve is applied.
    pub deadzone: f32,
    /// Response-curve exponent; `0.0` is linear. See `crate::normalize` for
    /// the exact formula.
    pub expo: f32,
    /// Calibration range mapping raw units to `[-1, 1]`.
    pub calibration: AxisCalibration,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{AxisCalibration, AxisConfig};

    #[test]
    fn axis_config_roundtrips_through_json() {
        let config = AxisConfig {
            source_index: 0,
            logical: "roll".to_string(),
            invert: false,
            deadzone: 0.05,
            expo: 0.3,
            calibration: AxisCalibration {
                min: -1.0,
                center: 0.0,
                max: 1.0,
            },
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let back: AxisConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, config);
    }
}

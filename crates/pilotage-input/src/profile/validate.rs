//! Per-axis calibration and response-curve validation (ADR-0007).
//!
//! The one place that decides whether an [`AxisConfig`]'s numbers define a
//! usable normalization: strict calibration ordering, and a deadzone/expo the
//! pipeline keeps monotonic and bounded. `validate_profile` calls it for every
//! axis, and the browser control runtime calls it for each scheme-bound axis,
//! so the native host and the WASM engine accept and reject identical configs.

use super::AxisConfig;
use super::error::ProfileError;

/// The response-curve limits the normalization pipeline is defined over: a
/// deadzone in `[0.0, 1.0)` and an expo in `[-0.99, 10.0]`.
const DEADZONE_RANGE: core::ops::Range<f32> = 0.0..1.0;
const EXPO_RANGE: core::ops::RangeInclusive<f32> = -0.99..=10.0;

/// Validates one axis's calibration and response curve.
///
/// The calibration must be finite and strictly ordered (`min < center < max`),
/// so each side of center has a positive span; the deadzone must be finite and
/// in `[0.0, 1.0)`; the expo must be finite and in `[-0.99, 10.0]`.
///
/// # Errors
///
/// Returns [`ProfileError::DegenerateCalibration`] for a non-finite or
/// out-of-order calibration, [`ProfileError::NonFiniteAxisValue`] for a
/// non-finite deadzone or expo, and [`ProfileError::DeadzoneOutOfRange`] /
/// [`ProfileError::ExpoOutOfRange`] for an out-of-range one.
pub fn validate_axis_config(axis: &AxisConfig) -> Result<(), ProfileError> {
    let source_index = axis.source_index;
    let cal = &axis.calibration;
    let ordered = cal.min.is_finite()
        && cal.center.is_finite()
        && cal.max.is_finite()
        && cal.min < cal.center
        && cal.center < cal.max;
    if !ordered {
        return Err(ProfileError::DegenerateCalibration {
            source_index,
            min: cal.min,
            center: cal.center,
            max: cal.max,
        });
    }
    if !axis.deadzone.is_finite() {
        return Err(ProfileError::NonFiniteAxisValue {
            source_index,
            field: "deadzone",
        });
    }
    if !DEADZONE_RANGE.contains(&axis.deadzone) {
        return Err(ProfileError::DeadzoneOutOfRange {
            source_index,
            value: axis.deadzone,
        });
    }
    if !axis.expo.is_finite() {
        return Err(ProfileError::NonFiniteAxisValue {
            source_index,
            field: "expo",
        });
    }
    if !EXPO_RANGE.contains(&axis.expo) {
        return Err(ProfileError::ExpoOutOfRange {
            source_index,
            value: axis.expo,
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::validate_axis_config;
    use crate::profile::{AxisCalibration, AxisConfig, ProfileError};

    fn axis(deadzone: f32, expo: f32, calibration: AxisCalibration) -> AxisConfig {
        AxisConfig {
            source_index: 2,
            logical: "yaw".to_owned(),
            invert: false,
            deadzone,
            expo,
            calibration,
        }
    }

    fn unit() -> AxisCalibration {
        AxisCalibration {
            min: -1.0,
            center: 0.0,
            max: 1.0,
        }
    }

    #[test]
    fn a_well_formed_axis_validates() {
        assert!(validate_axis_config(&axis(0.05, 0.35, unit())).is_ok());
    }

    #[test]
    fn a_degenerate_calibration_is_rejected() {
        let bad = AxisCalibration {
            min: 0.0,
            center: 0.0,
            max: 1.0,
        };
        assert!(matches!(
            validate_axis_config(&axis(0.0, 0.0, bad)),
            Err(ProfileError::DegenerateCalibration {
                source_index: 2,
                ..
            })
        ));
    }

    #[test]
    fn a_non_finite_calibration_is_rejected() {
        let bad = AxisCalibration {
            min: f32::NAN,
            center: 0.0,
            max: 1.0,
        };
        assert!(matches!(
            validate_axis_config(&axis(0.0, 0.0, bad)),
            Err(ProfileError::DegenerateCalibration { .. })
        ));
    }

    #[test]
    fn an_out_of_range_deadzone_is_rejected() {
        assert!(matches!(
            validate_axis_config(&axis(1.0, 0.0, unit())),
            Err(ProfileError::DeadzoneOutOfRange { value, .. }) if value == 1.0
        ));
    }

    #[test]
    fn an_out_of_range_expo_is_rejected() {
        assert!(matches!(
            validate_axis_config(&axis(0.0, 42.0, unit())),
            Err(ProfileError::ExpoOutOfRange { value, .. }) if value == 42.0
        ));
    }

    #[test]
    fn a_non_finite_response_curve_is_rejected() {
        assert!(matches!(
            validate_axis_config(&axis(f32::INFINITY, 0.0, unit())),
            Err(ProfileError::NonFiniteAxisValue {
                field: "deadzone",
                ..
            })
        ));
    }
}

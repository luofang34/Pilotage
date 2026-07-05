//! Pure axis normalization pipeline (ADR-0007).
//!
//! Stage order is fixed and documented here because later stages assume
//! earlier ones already ran:
//!
//! 1. **Calibration**: raw units -> `[-1, 1]` with `center` mapped to `0.0`,
//!    each side of center scaled independently so asymmetric ranges are
//!    handled without clipping the shorter side early.
//! 2. **Deadzone**: values within `deadzone` of `0.0` are clamped to `0.0`;
//!    values outside it are re-scaled so the output is continuous at the
//!    deadzone edge (no jump discontinuity when crossing in or out).
//! 3. **Expo curve**: reshapes response around center per the formula
//!    documented on [`apply_expo`].
//! 4. **Invert**: negates the value if the axis is configured inverted.
//! 5. **Clamp**: final clamp to `[-1, 1]` to absorb any floating-point
//!    overshoot from the previous stages.
//!
//! Non-finite raw input (`NaN`, `inf`) never propagates: [`normalize_axis`]
//! maps it to `0.0` and reports a fault flag instead.

use crate::profile::{AxisCalibration, AxisConfig};

/// Result of normalizing one raw axis sample through the full pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedAxis {
    /// Final value in `[-1.0, 1.0]`.
    pub value: f32,
    /// Set when the raw input was non-finite (`NaN` or infinite) and was
    /// therefore mapped to `0.0` instead of propagating.
    pub fault: bool,
}

/// Runs the full calibration -> deadzone -> expo -> invert -> clamp
/// pipeline on one raw axis value.
///
/// Non-finite `raw` (NaN or +/-infinity) short-circuits to
/// `NormalizedAxis { value: 0.0, fault: true }` without evaluating any
/// later stage.
#[must_use]
pub fn normalize_axis(raw: f32, config: &AxisConfig) -> NormalizedAxis {
    if !raw.is_finite() {
        return NormalizedAxis {
            value: 0.0,
            fault: true,
        };
    }
    let calibrated = apply_calibration(raw, &config.calibration);
    let deadzoned = apply_deadzone(calibrated, config.deadzone);
    let curved = apply_expo(deadzoned, config.expo);
    let inverted = if config.invert { -curved } else { curved };
    NormalizedAxis {
        value: inverted.clamp(-1.0, 1.0),
        fault: false,
    }
}

/// Maps `raw` onto `[-1, 1]` using `calibration`, with `center` at `0.0` and
/// each side of center scaled independently by its own span.
///
/// A degenerate or reversed span on one side (`max <= center` or
/// `min >= center`) maps every value on that side to `0.0` rather than
/// dividing by a non-positive span, which would otherwise flip the output
/// sign instead of merely failing to divide by zero. `validate_profile`
/// rejects such ranges at load time; this guard covers callers that build
/// `AxisCalibration` directly.
#[must_use]
fn apply_calibration(raw: f32, calibration: &AxisCalibration) -> f32 {
    if raw >= calibration.center {
        let span = calibration.max - calibration.center;
        if span <= 0.0 {
            return 0.0;
        }
        ((raw - calibration.center) / span).clamp(-1.0, 1.0)
    } else {
        let span = calibration.center - calibration.min;
        if span <= 0.0 {
            return 0.0;
        }
        ((raw - calibration.center) / span).clamp(-1.0, 1.0)
    }
}

/// Clamps `value` to `0.0` inside `[-deadzone, deadzone]`, and linearly
/// re-scales the remaining range outside the deadzone back onto `[-1, 1]`
/// so the output is continuous (no jump) at the deadzone edge.
///
/// A `deadzone >= 1.0` collapses the whole axis to `0.0`.
#[must_use]
fn apply_deadzone(value: f32, deadzone: f32) -> f32 {
    let deadzone = deadzone.clamp(0.0, 1.0);
    if deadzone >= 1.0 {
        return 0.0;
    }
    let magnitude = value.abs();
    if magnitude <= deadzone {
        return 0.0;
    }
    let sign = value.signum();
    let rescaled = (magnitude - deadzone) / (1.0 - deadzone);
    sign * rescaled.clamp(0.0, 1.0)
}

/// Applies an exponential response curve: `output = sign(x) * |x|^(1 + expo)`.
///
/// `expo == 0.0` is linear (identity). Positive `expo` flattens response
/// near center and steepens it near the extremes (finer control at small
/// deflections); this crate clamps `expo` to `[-0.99, 10.0]` so the exponent
/// `1 + expo` stays positive and bounded, keeping the curve monotonic and
/// finite.
#[must_use]
fn apply_expo(value: f32, expo: f32) -> f32 {
    let exponent = 1.0 + expo.clamp(-0.99, 10.0);
    let magnitude = value.abs();
    value.signum() * magnitude.powf(exponent)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{apply_calibration, apply_deadzone, apply_expo, normalize_axis};
    use crate::profile::{AxisCalibration, AxisConfig};

    fn config(calibration: AxisCalibration, deadzone: f32, expo: f32, invert: bool) -> AxisConfig {
        AxisConfig {
            source_index: 0,
            logical: "roll".to_string(),
            invert,
            deadzone,
            expo,
            calibration,
        }
    }

    #[test]
    fn calibration_maps_center_to_zero() {
        let calibration = AxisCalibration {
            min: -100.0,
            center: 10.0,
            max: 200.0,
        };
        assert_eq!(apply_calibration(10.0, &calibration), 0.0);
    }

    #[test]
    fn calibration_handles_asymmetric_range() {
        let calibration = AxisCalibration {
            min: -50.0,
            center: 0.0,
            max: 200.0,
        };
        assert_eq!(apply_calibration(200.0, &calibration), 1.0);
        assert_eq!(apply_calibration(-50.0, &calibration), -1.0);
        // Halfway to max on the long side is 0.5, not 0.25.
        assert!((apply_calibration(100.0, &calibration) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn calibration_clamps_values_outside_range() {
        let calibration = AxisCalibration {
            min: -10.0,
            center: 0.0,
            max: 10.0,
        };
        assert_eq!(apply_calibration(50.0, &calibration), 1.0);
        assert_eq!(apply_calibration(-50.0, &calibration), -1.0);
    }

    #[test]
    fn calibration_min_equals_center_does_not_divide_by_zero() {
        let calibration = AxisCalibration {
            min: 0.0,
            center: 0.0,
            max: 10.0,
        };
        // Below center with zero span on that side maps to 0.0, not NaN/inf.
        assert_eq!(apply_calibration(-5.0, &calibration), 0.0);
        assert!(apply_calibration(5.0, &calibration).is_finite());
    }

    #[test]
    fn calibration_reversed_range_does_not_flip_sign() {
        // An unvalidated, schema-illegal reversed range (center above max)
        // must not divide by a negative span and invert the output sign.
        let calibration = AxisCalibration {
            min: 0.0,
            center: 20.0,
            max: 10.0,
        };
        assert_eq!(apply_calibration(25.0, &calibration), 0.0);
    }

    #[test]
    fn calibration_min_above_max_does_not_flip_sign() {
        let calibration = AxisCalibration {
            min: 1.0,
            center: 0.0,
            max: -1.0,
        };
        assert_eq!(apply_calibration(0.5, &calibration), 0.0);
    }

    #[test]
    fn deadzone_clamps_small_values_to_zero() {
        assert_eq!(apply_deadzone(0.03, 0.05), 0.0);
        assert_eq!(apply_deadzone(-0.03, 0.05), 0.0);
    }

    #[test]
    fn deadzone_is_continuous_at_edge() {
        let deadzone = 0.1;
        let just_inside = apply_deadzone(0.1, deadzone);
        let just_outside = apply_deadzone(0.1 + 1e-6, deadzone);
        assert_eq!(just_inside, 0.0);
        assert!(just_outside.abs() < 1e-4);
    }

    #[test]
    fn deadzone_rescales_to_full_range_at_extreme() {
        assert!((apply_deadzone(1.0, 0.2) - 1.0).abs() < 1e-6);
        assert!((apply_deadzone(-1.0, 0.2) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn deadzone_of_one_collapses_axis() {
        assert_eq!(apply_deadzone(1.0, 1.0), 0.0);
        assert_eq!(apply_deadzone(-1.0, 1.5), 0.0);
    }

    #[test]
    fn expo_zero_is_identity() {
        assert!((apply_expo(0.5, 0.0) - 0.5).abs() < 1e-6);
        assert!((apply_expo(-0.5, 0.0) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn expo_preserves_sign_and_endpoints() {
        assert_eq!(apply_expo(1.0, 0.5), 1.0);
        assert_eq!(apply_expo(-1.0, 0.5), -1.0);
        assert_eq!(apply_expo(0.0, 0.5), 0.0);
    }

    #[test]
    fn nan_raw_input_maps_to_zero_with_fault() {
        let cfg = config(
            AxisCalibration {
                min: -1.0,
                center: 0.0,
                max: 1.0,
            },
            0.0,
            0.0,
            false,
        );
        let result = normalize_axis(f32::NAN, &cfg);
        assert_eq!(result.value, 0.0);
        assert!(result.fault);
    }

    #[test]
    fn infinite_raw_input_maps_to_zero_with_fault() {
        let cfg = config(
            AxisCalibration {
                min: -1.0,
                center: 0.0,
                max: 1.0,
            },
            0.0,
            0.0,
            false,
        );
        let result = normalize_axis(f32::INFINITY, &cfg);
        assert_eq!(result.value, 0.0);
        assert!(result.fault);
        let result_neg = normalize_axis(f32::NEG_INFINITY, &cfg);
        assert_eq!(result_neg.value, 0.0);
        assert!(result_neg.fault);
    }

    #[test]
    fn finite_input_never_sets_fault() {
        let cfg = config(
            AxisCalibration {
                min: -1.0,
                center: 0.0,
                max: 1.0,
            },
            0.0,
            0.0,
            false,
        );
        let result = normalize_axis(0.5, &cfg);
        assert!(!result.fault);
    }

    #[test]
    fn invert_flips_sign_after_curve() {
        let cfg = config(
            AxisCalibration {
                min: -1.0,
                center: 0.0,
                max: 1.0,
            },
            0.0,
            0.0,
            true,
        );
        let result = normalize_axis(0.5, &cfg);
        assert!(result.value < 0.0);
    }

    #[test]
    fn output_is_always_clamped_to_unit_range() {
        let cfg = config(
            AxisCalibration {
                min: -1.0,
                center: 0.0,
                max: 1.0,
            },
            0.0,
            0.0,
            false,
        );
        let result = normalize_axis(1000.0, &cfg);
        assert!(result.value <= 1.0 && result.value >= -1.0);
    }

    #[test]
    fn values_outside_calibration_range_clamp_before_deadzone_and_expo() {
        let cfg = config(
            AxisCalibration {
                min: -10.0,
                center: 0.0,
                max: 10.0,
            },
            0.1,
            0.5,
            false,
        );
        let result = normalize_axis(500.0, &cfg);
        assert!((result.value - 1.0).abs() < 1e-6);
    }
}

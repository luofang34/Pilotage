use crate::{MetricError, TimedValue};

use super::{integral_abs_linear, validate_timed_values, weighted_abs_percentile_linear};

#[test]
fn non_monotonic_time_has_typed_context() {
    let samples = [
        TimedValue {
            time_s: 1.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 1.0,
            value: 1.0,
        },
    ];
    assert!(matches!(
        validate_timed_values(&samples),
        Err(MetricError::NonMonotonicTime { index: 1, .. })
    ));
}

#[test]
fn absolute_integral_splits_a_zero_crossing() {
    assert!((integral_abs_linear(-1.0, 1.0, 2.0) - 1.0).abs() < 1e-12);
}

#[test]
fn linear_absolute_percentile_uses_segment_duration() {
    let samples = [
        TimedValue {
            time_s: 0.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 2.0,
            value: 2.0,
        },
    ];
    let percentile = weighted_abs_percentile_linear(&samples, 0.95);
    assert!((percentile - 1.9).abs() < 1e-12);
}

#[test]
fn absolute_percentile_preserves_a_zero_duration_atom() {
    let samples = [
        TimedValue {
            time_s: 0.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 0.95,
            value: 0.0,
        },
        TimedValue {
            time_s: 1.0,
            value: 1e100,
        },
    ];

    assert_eq!(weighted_abs_percentile_linear(&samples, 0.95), 0.0);
}

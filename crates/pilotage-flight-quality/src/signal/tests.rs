use super::{measure_jerk, measure_signal};
use crate::test_trace::sample_value;
use crate::{MetricError, TimedValue};

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

#[test]
fn a_constant_signal_has_exact_rms_p95_and_peak() {
    let samples = sample_value(20, 2.0, |_| -2.0);
    let metrics = measure_signal(&samples).expect("valid constant trace");

    assert_close(metrics.rms, 2.0);
    assert_close(metrics.p95_abs, 2.0);
    assert_close(metrics.peak_abs, 2.0);
}

#[test]
fn an_acceleration_ramp_has_exact_jerk_metrics() {
    let acceleration = sample_value(10, 2.0, |time| time);
    let metrics = measure_jerk(&acceleration).expect("valid acceleration trace");

    assert_close(metrics.peak_acceleration_mps2, 2.0);
    assert_close(metrics.peak_jerk_mps3, 1.0);
    assert_close(metrics.jerk_p95_mps3, 1.0);
    assert_close(metrics.jerk_rms_mps3, 1.0);
    assert_close(metrics.integrated_abs_jerk_mps2, 2.0);
}

#[test]
fn a_linear_jerk_metric_is_independent_of_input_sample_rate() {
    let slow = measure_jerk(&sample_value(10, 3.0, |time| 2.0 * time)).expect("valid slow trace");
    let fast = measure_jerk(&sample_value(100, 3.0, |time| 2.0 * time)).expect("valid fast trace");

    assert_close(slow.peak_jerk_mps3, fast.peak_jerk_mps3);
    assert_close(slow.jerk_rms_mps3, fast.jerk_rms_mps3);
    assert_close(slow.integrated_abs_jerk_mps2, fast.integrated_abs_jerk_mps2);
}

#[test]
fn a_linear_signal_percentile_is_independent_of_input_sample_rate() {
    let slow = measure_signal(&sample_value(10, 2.0, |time| time)).expect("valid slow trace");
    let fast = measure_signal(&sample_value(100, 2.0, |time| time)).expect("valid fast trace");

    assert_close(slow.p95_abs, 1.9);
    assert_close(slow.p95_abs, fast.p95_abs);
}

#[test]
fn a_zero_duration_atom_has_an_exact_zero_p95() {
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
    let metrics = measure_signal(&samples).expect("finite large-dynamic-range signal");

    assert_eq!(metrics.p95_abs, 0.0);
}

#[test]
fn a_non_finite_derived_signal_metric_is_a_typed_error() {
    let samples = [
        TimedValue {
            time_s: 0.0,
            value: f64::MAX,
        },
        TimedValue {
            time_s: 1.0,
            value: f64::MAX,
        },
    ];

    assert_eq!(
        measure_signal(&samples),
        Err(MetricError::NonFiniteResult {
            field: "signal.rms",
        })
    );
}

#[test]
fn a_non_finite_derived_jerk_metric_is_a_typed_error() {
    let samples = [
        TimedValue {
            time_s: 0.0,
            value: f64::MAX,
        },
        TimedValue {
            time_s: 1.0,
            value: -f64::MAX,
        },
    ];

    assert!(matches!(
        measure_jerk(&samples),
        Err(MetricError::NonFiniteResult { .. })
    ));
}

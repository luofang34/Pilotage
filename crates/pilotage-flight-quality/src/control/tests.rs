use super::measure_control;
use crate::{ControlPoint, MetricError};

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

#[test]
fn a_control_pulse_has_exact_effort_and_saturation_metrics() {
    let samples = [
        ControlPoint {
            time_s: 0.0,
            effort: 0.0,
            saturated: false,
        },
        ControlPoint {
            time_s: 1.0,
            effort: 1.0,
            saturated: true,
        },
        ControlPoint {
            time_s: 2.0,
            effort: 1.0,
            saturated: true,
        },
        ControlPoint {
            time_s: 3.0,
            effort: 0.0,
            saturated: false,
        },
    ];
    let metrics = measure_control(&samples).expect("valid control trace");

    assert_close(metrics.effort_rms, (5.0_f64 / 9.0).sqrt());
    assert_close(metrics.integrated_abs_effort_s, 2.0);
    assert_close(metrics.saturation_fraction, 2.0 / 3.0);
    assert_close(metrics.longest_saturation_s, 2.0);
}

#[test]
fn separated_saturation_intervals_do_not_merge() {
    let samples = [
        ControlPoint {
            time_s: 0.0,
            effort: 0.5,
            saturated: true,
        },
        ControlPoint {
            time_s: 1.0,
            effort: 0.5,
            saturated: false,
        },
        ControlPoint {
            time_s: 2.0,
            effort: 0.5,
            saturated: true,
        },
        ControlPoint {
            time_s: 2.5,
            effort: 0.5,
            saturated: false,
        },
    ];
    let metrics = measure_control(&samples).expect("valid control trace");

    assert_close(metrics.longest_saturation_s, 1.0);
}

#[test]
fn a_non_finite_derived_control_metric_is_a_typed_error() {
    let samples = [
        ControlPoint {
            time_s: 0.0,
            effort: f64::MAX,
            saturated: false,
        },
        ControlPoint {
            time_s: 1.0,
            effort: f64::MAX,
            saturated: false,
        },
    ];

    assert_eq!(
        measure_control(&samples),
        Err(MetricError::NonFiniteResult {
            field: "control.effort_rms",
        })
    );
}

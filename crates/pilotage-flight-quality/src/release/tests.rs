use super::{measure_hold, measure_release};
use crate::{MetricError, MotionPoint, TimedValue};

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

#[test]
fn a_linear_brake_has_exact_stop_time_and_distance() {
    let motion = (0..=40)
        .map(|index| {
            let time_s = f64::from(index) / 10.0;
            let velocity = (2.0 - time_s).max(0.0);
            let position = if time_s <= 2.0 {
                2.0 * time_s - 0.5 * time_s * time_s
            } else {
                2.0
            };
            MotionPoint {
                time_s,
                position_m: position,
                velocity_mps: velocity,
            }
        })
        .collect::<Vec<_>>();
    let metrics = measure_release(&motion, 0.0, 3.0).expect("valid brake trace");

    assert_close(metrics.release_to_stop_s.expect("stop"), 1.95);
    assert_close(metrics.brake_distance_m.expect("distance"), 1.9975);
    assert_eq!(metrics.return_toward_release_m, 0.0);
    assert_eq!(metrics.opposite_velocity_peak_mps, 0.0);
}

#[test]
fn release_return_is_separate_from_final_hold_rebound() {
    let motion = vec![
        MotionPoint {
            time_s: 0.0,
            position_m: 0.0,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 1.0,
            position_m: 2.0,
            velocity_mps: 0.0,
        },
        MotionPoint {
            time_s: 2.0,
            position_m: 1.5,
            velocity_mps: -0.5,
        },
        MotionPoint {
            time_s: 3.0,
            position_m: 1.5,
            velocity_mps: 0.0,
        },
    ];
    let release = measure_release(&motion, 0.0, 2.0).expect("valid release trace");
    assert_close(release.return_toward_release_m, 0.5);
    assert_close(release.opposite_velocity_peak_mps, 0.5);

    let hold = [
        TimedValue {
            time_s: 2.0,
            value: 1.7,
        },
        TimedValue {
            time_s: 2.5,
            value: 1.4,
        },
        TimedValue {
            time_s: 3.0,
            value: 1.55,
        },
        TimedValue {
            time_s: 3.5,
            value: 1.48,
        },
        TimedValue {
            time_s: 4.0,
            value: 1.5,
        },
    ];
    let rebound = measure_hold(&hold, 2.0, 1.5).expect("valid hold trace");
    assert_close(rebound.rebound_distance_m, 0.1);
    assert_eq!(rebound.zero_crossings, 3);
}

#[test]
fn the_zero_crossing_hysteresis_ignores_center_noise() {
    let position = [
        TimedValue {
            time_s: 0.0,
            value: 0.2,
        },
        TimedValue {
            time_s: 1.0,
            value: 0.005,
        },
        TimedValue {
            time_s: 2.0,
            value: -0.004,
        },
        TimedValue {
            time_s: 3.0,
            value: -0.1,
        },
    ];
    let metrics = measure_hold(&position, 0.0, 0.0).expect("valid hold trace");

    assert_eq!(metrics.zero_crossings, 1);
    assert_close(metrics.rebound_distance_m, 0.1);
}

#[test]
fn release_uses_the_largest_return_and_the_first_opposite_lobe() {
    let motion = [
        MotionPoint {
            time_s: 0.0,
            position_m: 0.0,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 1.0,
            position_m: 2.0,
            velocity_mps: -0.4,
        },
        MotionPoint {
            time_s: 2.0,
            position_m: 1.0,
            velocity_mps: 0.2,
        },
        MotionPoint {
            time_s: 3.0,
            position_m: 1.5,
            velocity_mps: -0.8,
        },
        MotionPoint {
            time_s: 4.0,
            position_m: 1.5,
            velocity_mps: 0.0,
        },
    ];
    let metrics = measure_release(&motion, 0.0, 3.5).expect("valid release trace");

    assert_close(metrics.return_toward_release_m, 1.0);
    assert_close(metrics.opposite_velocity_peak_mps, 0.4);
}

#[test]
fn interpolated_event_boundaries_contribute_to_opposite_velocity() {
    let motion = [
        MotionPoint {
            time_s: 0.0,
            position_m: 0.0,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 2.0,
            position_m: 1.0,
            velocity_mps: -1.0,
        },
    ];
    let metrics = measure_release(&motion, 0.5, 1.5).expect("valid release trace");

    assert_close(metrics.opposite_velocity_peak_mps, 0.5);
}

#[test]
fn a_stop_after_hold_entry_does_not_change_release_metrics() {
    let motion = [
        MotionPoint {
            time_s: 0.0,
            position_m: 0.0,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 1.0,
            position_m: 1.0,
            velocity_mps: 0.2,
        },
        MotionPoint {
            time_s: 2.0,
            position_m: 1.2,
            velocity_mps: 0.0,
        },
        MotionPoint {
            time_s: 3.0,
            position_m: 1.2,
            velocity_mps: 0.0,
        },
    ];
    let metrics = measure_release(&motion, 0.0, 1.0).expect("valid release phase");

    assert_eq!(metrics.release_to_stop_s, None);
    assert_eq!(metrics.brake_distance_m, None);
}

#[test]
fn a_zero_duration_hold_is_not_evidence() {
    let position = [
        TimedValue {
            time_s: 0.0,
            value: 1.0,
        },
        TimedValue {
            time_s: 1.0,
            value: 0.0,
        },
    ];

    assert_eq!(
        measure_hold(&position, 1.0, 0.0),
        Err(MetricError::InvalidParameter {
            field: "hold_start_s",
        })
    );
}

#[test]
fn a_non_finite_derived_release_metric_is_a_typed_error() {
    let motion = [
        MotionPoint {
            time_s: 0.0,
            position_m: f64::MAX,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 1.0,
            position_m: -f64::MAX,
            velocity_mps: 1.0,
        },
    ];

    assert!(matches!(
        measure_release(&motion, 0.0, 1.0),
        Err(MetricError::NonFiniteResult { .. })
    ));
}

#[test]
fn a_non_finite_hold_error_is_a_typed_error() {
    let position = [
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
        measure_hold(&position, 0.0, -f64::MAX),
        Err(MetricError::NonFiniteResult {
            field: "hold.position_error_m",
        })
    );
}

#[test]
fn signed_zero_release_and_hold_times_select_the_first_sample() {
    let motion = [
        MotionPoint {
            time_s: 0.0,
            position_m: 0.0,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 1.0,
            position_m: 0.5,
            velocity_mps: 0.0,
        },
        MotionPoint {
            time_s: 2.0,
            position_m: 0.5,
            velocity_mps: 0.0,
        },
    ];
    let release = measure_release(&motion, -0.0, 1.5)
        .expect("signed zero identifies the first release sample");
    assert!(release.release_to_stop_s.is_some());

    let position = [
        TimedValue {
            time_s: 0.0,
            value: 0.2,
        },
        TimedValue {
            time_s: 1.0,
            value: 0.0,
        },
    ];
    let hold =
        measure_hold(&position, -0.0, 0.0).expect("signed zero identifies the first hold sample");
    assert_eq!(hold.zero_crossings, 0);
}

#[test]
fn non_finite_stop_velocity_delta_has_typed_context() {
    let motion = [
        MotionPoint {
            time_s: 0.0,
            position_m: 0.0,
            velocity_mps: f64::MAX,
        },
        MotionPoint {
            time_s: 1.0,
            position_m: 0.0,
            velocity_mps: -f64::MAX,
        },
    ];

    assert_eq!(
        measure_release(&motion, 0.0, 1.0),
        Err(MetricError::NonFiniteResult {
            field: "release.stop_velocity_delta_mps",
        })
    );
}

#[test]
fn non_finite_release_displacement_has_typed_context() {
    let motion = [
        MotionPoint {
            time_s: 0.0,
            position_m: -f64::MAX,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 1.0,
            position_m: f64::MAX,
            velocity_mps: 1.0,
        },
    ];

    assert_eq!(
        measure_release(&motion, 0.0, 1.0),
        Err(MetricError::NonFiniteResult {
            field: "release.displacement_m",
        })
    );
}

#[test]
fn non_finite_release_return_distance_has_typed_context() {
    let motion = [
        MotionPoint {
            time_s: 0.0,
            position_m: 0.0,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 1.0,
            position_m: f64::MAX,
            velocity_mps: 1.0,
        },
        MotionPoint {
            time_s: 2.0,
            position_m: -f64::MAX,
            velocity_mps: 1.0,
        },
    ];

    assert_eq!(
        measure_release(&motion, 0.0, 2.0),
        Err(MetricError::NonFiniteResult {
            field: "release.return_distance_m",
        })
    );
}

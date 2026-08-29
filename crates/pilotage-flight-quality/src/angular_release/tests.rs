use core::f64::consts::PI;

use super::*;
use crate::test_trace::sample_value;

const RATE_HZ: u32 = 200;
const DEGREE: f64 = PI / 180.0;

/// A return from a departure angle that overshoots the target by one peak.
fn return_with_peak(departure_rad: f64, peak_rad: f64) -> Vec<TimedValue> {
    sample_value(RATE_HZ, 4.0, |time_s| {
        if time_s < 1.0 {
            departure_rad
        } else if time_s < 1.5 {
            departure_rad + (peak_rad - departure_rad) * (time_s - 1.0) / 0.5
        } else if time_s < 2.0 {
            peak_rad * (2.0 - time_s) / 0.5
        } else {
            0.0
        }
    })
}

#[test]
fn the_opposite_peak_is_the_excursion_past_the_return_target() {
    let attitude = return_with_peak(10.0 * DEGREE, -0.8 * DEGREE);
    let rate = sample_value(RATE_HZ, 4.0, |_| 0.0);
    let metrics = measure_angular_release(
        &attitude,
        &rate,
        AngularReleaseSpec {
            release_time_s: 1.0,
            departure_rad: 10.0 * DEGREE,
            return_target_rad: 0.0,
        },
    )
    .expect("the return is measurable");
    assert!(
        (metrics.opposite_return_peak_rad - 0.8 * DEGREE).abs() < 1.0e-6,
        "peak was {} rad",
        metrics.opposite_return_peak_rad
    );
}

#[test]
fn a_return_that_stops_short_states_no_opposite_peak() {
    let attitude = return_with_peak(10.0 * DEGREE, 1.0 * DEGREE);
    let rate = sample_value(RATE_HZ, 4.0, |_| 0.0);
    let metrics = measure_angular_release(
        &attitude,
        &rate,
        AngularReleaseSpec {
            release_time_s: 1.0,
            departure_rad: 10.0 * DEGREE,
            return_target_rad: 0.0,
        },
    )
    .expect("the return is measurable");
    assert!(metrics.opposite_return_peak_rad.abs() < 1.0e-12);
}

#[test]
fn the_opposite_side_follows_the_departure_side() {
    // A vehicle that departed negative overshoots positive, and the same
    // measurement has to find it there.
    let attitude = return_with_peak(-10.0 * DEGREE, 0.8 * DEGREE);
    let rate = sample_value(RATE_HZ, 4.0, |_| 0.0);
    let metrics = measure_angular_release(
        &attitude,
        &rate,
        AngularReleaseSpec {
            release_time_s: 1.0,
            departure_rad: -10.0 * DEGREE,
            return_target_rad: 0.0,
        },
    )
    .expect("the return is measurable");
    assert!((metrics.opposite_return_peak_rad - 0.8 * DEGREE).abs() < 1.0e-6);
}

#[test]
fn a_return_across_the_wrap_is_one_small_excursion() {
    // The vehicle departs to 175 degrees, returns to 179, and passes it by one
    // degree onto minus 180. Plain subtraction would call that a 359 degree
    // excursion.
    let attitude = sample_value(RATE_HZ, 3.0, |time_s| {
        if time_s < 1.0 {
            175.0 * DEGREE
        } else if time_s < 1.5 {
            -180.0 * DEGREE
        } else {
            179.0 * DEGREE
        }
    });
    let rate = sample_value(RATE_HZ, 3.0, |_| 0.0);
    let metrics = measure_angular_release(
        &attitude,
        &rate,
        AngularReleaseSpec {
            release_time_s: 1.0,
            departure_rad: 175.0 * DEGREE,
            return_target_rad: 179.0 * DEGREE,
        },
    )
    .expect("the wrapped return is measurable");
    assert!(
        metrics.opposite_return_peak_rad < 2.0 * DEGREE,
        "peak was {} rad",
        metrics.opposite_return_peak_rad
    );
}

#[test]
fn the_final_rate_root_mean_square_covers_the_last_second() {
    // Loud for the first two seconds, quiet for the last one. Only the last
    // second counts, so the result is the quiet value.
    let attitude = return_with_peak(10.0 * DEGREE, 0.0);
    let rate = sample_value(RATE_HZ, 4.0, |time_s| if time_s < 3.0 { 5.0 } else { 0.25 });
    let metrics = measure_angular_release(
        &attitude,
        &rate,
        AngularReleaseSpec {
            release_time_s: 1.0,
            departure_rad: 10.0 * DEGREE,
            return_target_rad: 0.0,
        },
    )
    .expect("the return is measurable");
    assert!(
        (metrics.final_body_rate_rms_rps - 0.25).abs() < 1.0e-6,
        "rms was {} rad/s",
        metrics.final_body_rate_rms_rps
    );
}

#[test]
fn a_constant_rate_root_mean_square_is_that_rate() {
    let attitude = return_with_peak(10.0 * DEGREE, 0.0);
    let rate = sample_value(RATE_HZ, 4.0, |_| 0.5 * DEGREE);
    let metrics = measure_angular_release(
        &attitude,
        &rate,
        AngularReleaseSpec {
            release_time_s: 1.0,
            departure_rad: 10.0 * DEGREE,
            return_target_rad: 0.0,
        },
    )
    .expect("the return is measurable");
    assert!((metrics.final_body_rate_rms_rps - 0.5 * DEGREE).abs() < 1.0e-9);
}

#[test]
fn a_negative_body_rate_magnitude_is_refused() {
    let attitude = return_with_peak(10.0 * DEGREE, 0.0);
    let rate = sample_value(RATE_HZ, 4.0, |_| -0.1);
    let error = measure_angular_release(
        &attitude,
        &rate,
        AngularReleaseSpec {
            release_time_s: 1.0,
            departure_rad: 10.0 * DEGREE,
            return_target_rad: 0.0,
        },
    )
    .expect_err("a magnitude is never negative");
    assert!(matches!(error, MetricError::InvalidParameter { .. }));
}

#[test]
fn a_departure_equal_to_the_return_target_is_refused() {
    let attitude = return_with_peak(0.0, 0.0);
    let rate = sample_value(RATE_HZ, 4.0, |_| 0.0);
    let error = measure_angular_release(
        &attitude,
        &rate,
        AngularReleaseSpec {
            release_time_s: 1.0,
            departure_rad: 0.0,
            return_target_rad: 0.0,
        },
    )
    .expect_err("a return with no departure has no opposite side");
    assert!(matches!(error, MetricError::ZeroStep));
}

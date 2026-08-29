use super::*;
use crate::test_trace::sample_value;

const RATE_HZ: u32 = 200;

/// A down acceleration that answers a collective increase by rising upward.
fn obedient_trace(step_at_s: f64, magnitude_mps2: f64) -> Vec<TimedValue> {
    sample_value(RATE_HZ, 3.0, |time_s| {
        if time_s < step_at_s {
            0.0
        } else {
            // Up is a negative down component.
            -magnitude_mps2 * (1.0 - (-(time_s - step_at_s) / 0.1).exp())
        }
    })
}

#[test]
fn more_collective_answered_by_upward_acceleration_reads_positive() {
    let acceleration = obedient_trace(0.5, 2.0);
    let metrics = measure_collective_response(
        &acceleration,
        CollectiveStepSpec {
            input_time_s: 0.5,
            baseline_force: 1.0,
            target_force: 1.2,
        },
    )
    .expect("the collective step is measurable");
    assert!((metrics.commanded_force_delta - 0.2).abs() < 1.0e-12);
    assert!(
        (metrics.peak_response_mps2 - 2.0).abs() < 0.02,
        "peak was {} m/s2",
        metrics.peak_response_mps2
    );
    assert!(metrics.direction_error_fraction < 1.0e-9);
    assert!(metrics.input_to_response_delay_s.is_some());
}

#[test]
fn less_collective_answered_by_downward_acceleration_also_reads_positive() {
    // The vehicle sinks, which is a positive down component, and the command
    // asked it to. The measurement is of obedience, not of sign.
    let acceleration = sample_value(RATE_HZ, 3.0, |time_s| if time_s < 0.5 { 0.0 } else { 1.5 });
    let metrics = measure_collective_response(
        &acceleration,
        CollectiveStepSpec {
            input_time_s: 0.5,
            baseline_force: 1.0,
            target_force: 0.8,
        },
    )
    .expect("the collective step is measurable");
    assert!((metrics.peak_response_mps2 - 1.5).abs() < 1.0e-9);
    assert!(metrics.direction_error_fraction < 1.0e-9);
}

#[test]
fn a_vehicle_that_accelerates_the_wrong_way_states_the_time_it_spent_there() {
    // Half the window is spent going the wrong way.
    let acceleration = sample_value(RATE_HZ, 2.5, |time_s| {
        if time_s < 0.5 {
            0.0
        } else if time_s < 1.5 {
            // Sinking while asked to climb.
            1.0
        } else {
            -1.0
        }
    });
    let metrics = measure_collective_response(
        &acceleration,
        CollectiveStepSpec {
            input_time_s: 0.5,
            baseline_force: 1.0,
            target_force: 1.3,
        },
    )
    .expect("the collective step is measurable");
    assert!(
        (metrics.direction_error_fraction - 0.5).abs() < 0.02,
        "fraction was {}",
        metrics.direction_error_fraction
    );
}

#[test]
fn the_steady_response_reads_the_final_window_not_the_peak() {
    let acceleration = sample_value(RATE_HZ, 3.0, |time_s| {
        if time_s < 0.5 {
            0.0
        } else if time_s < 1.0 {
            -4.0
        } else {
            -1.0
        }
    });
    let metrics = measure_collective_response(
        &acceleration,
        CollectiveStepSpec {
            input_time_s: 0.5,
            baseline_force: 1.0,
            target_force: 1.2,
        },
    )
    .expect("the collective step is measurable");
    assert!((metrics.peak_response_mps2 - 4.0).abs() < 1.0e-9);
    assert!((metrics.steady_response_mps2 - 1.0).abs() < 1.0e-9);
}

#[test]
fn a_collective_request_that_changes_nothing_is_refused() {
    let acceleration = obedient_trace(0.5, 2.0);
    let error = measure_collective_response(
        &acceleration,
        CollectiveStepSpec {
            input_time_s: 0.5,
            baseline_force: 1.0,
            target_force: 1.0,
        },
    )
    .expect_err("a zero force change has no response");
    assert!(matches!(error, MetricError::ZeroStep));
}

#[test]
fn a_vehicle_that_never_answers_states_no_positive_peak() {
    let acceleration = sample_value(RATE_HZ, 3.0, |_| 0.0);
    let metrics = measure_collective_response(
        &acceleration,
        CollectiveStepSpec {
            input_time_s: 0.5,
            baseline_force: 1.0,
            target_force: 1.2,
        },
    )
    .expect("the collective step is measurable");
    assert!(metrics.peak_response_mps2.abs() < 1.0e-12);
    assert_eq!(metrics.input_to_response_delay_s, None);
}

#[test]
fn an_input_event_outside_the_series_is_refused() {
    let acceleration = obedient_trace(0.5, 2.0);
    let error = measure_collective_response(
        &acceleration,
        CollectiveStepSpec {
            input_time_s: 30.0,
            baseline_force: 1.0,
            target_force: 1.2,
        },
    )
    .expect_err("an event outside the series has no window");
    assert!(matches!(error, MetricError::EventOutsideSeries { .. }));
}

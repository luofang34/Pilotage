use super::{StepSpec, measure_step_response};
use crate::test_trace::sample_value;
use crate::{MetricError, TimedValue};

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

#[test]
fn a_linear_step_has_exact_delay_rise_and_settle_metrics() {
    let command = sample_value(10, 4.0, |time| ((time - 1.2) / 1.0).clamp(0.0, 1.0));
    let response = sample_value(10, 4.0, |time| ((time - 1.5) / 2.0).clamp(0.0, 1.0));
    let metrics = measure_step_response(
        &command,
        &response,
        StepSpec {
            input_time_s: 1.0,
            initial_value: 0.0,
            target_value: 1.0,
        },
    )
    .expect("valid analytic trace");

    assert_close(
        metrics.input_to_command_delay_s.expect("command delay"),
        0.22,
    );
    assert_close(
        metrics.input_to_response_delay_s.expect("response delay"),
        0.54,
    );
    assert_close(metrics.rise_time_s.expect("rise"), 1.6);
    assert_close(metrics.settling_time_s.expect("settle"), 2.4);
    assert_eq!(metrics.overshoot, 0.0);
    assert_eq!(metrics.undershoot, 0.0);
    assert_close(metrics.steady_state_error, 0.0);
    assert_close(metrics.integrated_absolute_error, 1.5);
}

#[test]
fn a_damped_shape_reports_overshoot_and_final_settle_entry() {
    let command = vec![
        TimedValue {
            time_s: 0.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 1.0,
            value: 1.0,
        },
        TimedValue {
            time_s: 4.0,
            value: 1.0,
        },
    ];
    let response = vec![
        TimedValue {
            time_s: 0.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 1.0,
            value: 1.2,
        },
        TimedValue {
            time_s: 2.0,
            value: 0.9,
        },
        TimedValue {
            time_s: 3.0,
            value: 1.0,
        },
        TimedValue {
            time_s: 4.0,
            value: 1.0,
        },
    ];
    let metrics = measure_step_response(
        &command,
        &response,
        StepSpec {
            input_time_s: 0.0,
            initial_value: 0.0,
            target_value: 1.0,
        },
    )
    .expect("valid analytic trace");

    assert_close(metrics.overshoot, 0.2);
    assert_close(metrics.overshoot_fraction, 0.2);
    assert_close(metrics.settling_time_s.expect("settle"), 2.5);
}

#[test]
fn linear_metrics_are_independent_of_input_sample_rate() {
    let run = |rate_hz| {
        let series = sample_value(rate_hz, 2.0, |time| ((time - 0.2) / 1.0).clamp(0.0, 1.0));
        measure_step_response(
            &series,
            &series,
            StepSpec {
                input_time_s: 0.0,
                initial_value: 0.0,
                target_value: 1.0,
            },
        )
        .expect("valid analytic trace")
    };
    let slow = run(10);
    let fast = run(100);

    assert_close(
        slow.input_to_response_delay_s.expect("slow delay"),
        fast.input_to_response_delay_s.expect("fast delay"),
    );
    assert_close(
        slow.rise_time_s.expect("slow rise"),
        fast.rise_time_s.expect("fast rise"),
    );
    assert_close(
        slow.settling_time_s.expect("slow settle"),
        fast.settling_time_s.expect("fast settle"),
    );
    assert_close(
        slow.integrated_absolute_error,
        fast.integrated_absolute_error,
    );
}

#[test]
fn a_non_finite_trace_fails_with_sample_context() {
    let bad = [
        TimedValue {
            time_s: 0.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 1.0,
            value: f64::NAN,
        },
    ];
    let error = measure_step_response(
        &bad,
        &bad,
        StepSpec {
            input_time_s: 0.0,
            initial_value: 0.0,
            target_value: 1.0,
        },
    )
    .expect_err("non-finite value");

    assert_eq!(
        error,
        MetricError::NonFiniteValue {
            index: 1,
            field: "value",
        }
    );
}

#[test]
fn response_result_json_is_byte_stable() {
    let metrics = super::ResponseMetrics {
        input_to_command_delay_s: Some(0.1),
        input_to_response_delay_s: Some(0.2),
        rise_time_s: Some(0.3),
        settling_time_s: None,
        overshoot: 0.4,
        overshoot_fraction: 0.5,
        undershoot: 0.6,
        steady_state_error: -0.1,
        integrated_absolute_error: 0.7,
    };
    let bytes = serde_json::to_vec(&metrics).expect("serialize response metrics");

    assert_eq!(
        bytes,
        br#"{"input_to_command_delay_s":0.1,"input_to_response_delay_s":0.2,"rise_time_s":0.3,"settling_time_s":null,"overshoot":0.4,"overshoot_fraction":0.5,"undershoot":0.6,"steady_state_error":-0.1,"integrated_absolute_error":0.7}"#
    );
    let decoded: super::ResponseMetrics =
        serde_json::from_slice(&bytes).expect("deserialize response metrics");
    assert_eq!(decoded, metrics);
}

#[test]
fn a_non_finite_derived_response_metric_is_a_typed_error() {
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
        measure_step_response(
            &samples,
            &samples,
            StepSpec {
                input_time_s: 0.0,
                initial_value: f64::MAX,
                target_value: -f64::MAX,
            },
        ),
        Err(MetricError::NonFiniteResult {
            field: "response.step_delta",
        })
    );
}

#[test]
fn signed_zero_input_time_is_the_first_step_sample() {
    let samples = [
        TimedValue {
            time_s: 0.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 1.0,
            value: 1.0,
        },
    ];

    let metrics = measure_step_response(
        &samples,
        &samples,
        StepSpec {
            input_time_s: -0.0,
            initial_value: 0.0,
            target_value: 1.0,
        },
    )
    .expect("signed zero identifies the first sample");

    assert_eq!(metrics.input_to_command_delay_s, Some(0.02));
    assert_eq!(metrics.input_to_response_delay_s, Some(0.02));
}

#[test]
fn non_finite_command_progress_has_typed_context() {
    let command = [
        TimedValue {
            time_s: 0.0,
            value: -f64::MAX,
        },
        TimedValue {
            time_s: 1.0,
            value: f64::MAX,
        },
    ];
    let response = [
        TimedValue {
            time_s: 0.0,
            value: -f64::MAX,
        },
        TimedValue {
            time_s: 1.0,
            value: 0.0,
        },
    ];

    assert_eq!(
        measure_step_response(
            &command,
            &response,
            StepSpec {
                input_time_s: 0.0,
                initial_value: -f64::MAX,
                target_value: 0.0,
            },
        ),
        Err(MetricError::NonFiniteValue {
            index: 1,
            field: "response.command_progress",
        })
    );
}

#[test]
fn non_finite_response_progress_has_typed_context() {
    let command = [
        TimedValue {
            time_s: 0.0,
            value: -f64::MAX,
        },
        TimedValue {
            time_s: 1.0,
            value: 0.0,
        },
    ];
    let response = [
        TimedValue {
            time_s: 0.0,
            value: -f64::MAX,
        },
        TimedValue {
            time_s: 1.0,
            value: f64::MAX,
        },
    ];

    assert_eq!(
        measure_step_response(
            &command,
            &response,
            StepSpec {
                input_time_s: 0.0,
                initial_value: -f64::MAX,
                target_value: 0.0,
            },
        ),
        Err(MetricError::NonFiniteValue {
            index: 1,
            field: "response.response_progress",
        })
    );
}

#[test]
fn non_finite_response_error_has_typed_context() {
    let command = [
        TimedValue {
            time_s: 0.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 1.0,
            value: -f64::MAX,
        },
    ];
    let response = [
        TimedValue {
            time_s: 0.0,
            value: 0.0,
        },
        TimedValue {
            time_s: 1.0,
            value: f64::MAX,
        },
    ];

    assert_eq!(
        measure_step_response(
            &command,
            &response,
            StepSpec {
                input_time_s: 0.0,
                initial_value: 0.0,
                target_value: -f64::MAX,
            },
        ),
        Err(MetricError::NonFiniteValue {
            index: 1,
            field: "response.error",
        })
    );
}

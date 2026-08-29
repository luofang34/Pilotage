use core::f64::consts::PI;

use super::*;
use crate::test_trace::sample_value;

const RATE_HZ: u32 = 200;
const DEGREE: f64 = PI / 180.0;

/// A first-order approach from a baseline to a target angle.
fn first_order(baseline_rad: f64, target_rad: f64, tau_s: f64, step_at_s: f64) -> Vec<TimedValue> {
    let amplitude = shortest_arc_rad(target_rad - baseline_rad);
    sample_value(RATE_HZ, 4.0, |time_s| {
        if time_s < step_at_s {
            baseline_rad
        } else {
            let progress = 1.0 - (-(time_s - step_at_s) / tau_s).exp();
            baseline_rad + amplitude * progress
        }
    })
}

#[test]
fn a_step_across_the_wrap_measures_the_short_arc() {
    // Holding just under half a turn and being asked for just over it is a
    // two degree request, not a 358 degree one.
    let baseline = 179.0 * DEGREE;
    let target = -179.0 * DEGREE;
    let attitude = first_order(baseline, target, 0.15, 0.5);
    let metrics = measure_angular_step(
        &attitude,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: baseline,
            target_rad: target,
        },
    )
    .expect("the wrapped step is measurable");
    assert!(
        (metrics.amplitude_rad - 2.0 * DEGREE).abs() < 1.0e-9,
        "amplitude was {} rad",
        metrics.amplitude_rad
    );
    assert!(metrics.settling_time_s.is_some());
    assert!(metrics.overshoot_fraction.abs() < 1.0e-9);
}

#[test]
fn a_runtime_resolved_baseline_fixes_the_step_amplitude() {
    // The same trace measured against two baselines states two different
    // amplitudes, which is why the baseline may not be a scenario constant.
    let attitude = first_order(4.0 * DEGREE, 14.0 * DEGREE, 0.1, 0.5);
    let resolved = measure_angular_step(
        &attitude,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: 4.0 * DEGREE,
            target_rad: 14.0 * DEGREE,
        },
    )
    .expect("the resolved baseline is measurable");
    let assumed_zero = measure_angular_step(
        &attitude,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: 0.0,
            target_rad: 14.0 * DEGREE,
        },
    )
    .expect("the assumed baseline is measurable");
    assert!((resolved.amplitude_rad - 10.0 * DEGREE).abs() < 1.0e-9);
    assert!((assumed_zero.amplitude_rad - 14.0 * DEGREE).abs() < 1.0e-9);
    // A static zero baseline reads the trace as already 29 percent of the way
    // through its step at the input event, so the rise it reports is not the
    // rise the vehicle flew.
    assert!(resolved.rise_time_s > assumed_zero.rise_time_s);
}

#[test]
fn the_settling_band_is_five_percent_of_the_physical_amplitude() {
    // A trace that parks 4 percent short of a ten degree target has settled;
    // the same absolute error against a two degree target has not.
    let near = sample_value(
        RATE_HZ,
        3.0,
        |time_s| {
            if time_s < 0.5 { 0.0 } else { 9.6 * DEGREE }
        },
    );
    let inside = measure_angular_step(
        &near,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: 0.0,
            target_rad: 10.0 * DEGREE,
        },
    )
    .expect("the near trace is measurable");
    assert!(inside.settling_time_s.is_some());

    let far = sample_value(
        RATE_HZ,
        3.0,
        |time_s| {
            if time_s < 0.5 { 0.0 } else { 1.6 * DEGREE }
        },
    );
    let outside = measure_angular_step(
        &far,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: 0.0,
            target_rad: 2.0 * DEGREE,
        },
    )
    .expect("the far trace is measurable");
    assert_eq!(outside.settling_time_s, None);
}

#[test]
fn a_ten_degree_step_states_its_settling_time_and_overshoot() {
    // A trace built to enter the band at exactly 1.01 seconds after the input
    // reports that time, which is what a scoped limit of one second refuses.
    let attitude = sample_value(RATE_HZ, 4.0, |time_s| {
        if time_s < 0.5 {
            0.0
        } else if time_s < 1.51 {
            // Sits 6 percent high, outside the 5 percent band.
            10.6 * DEGREE
        } else {
            10.0 * DEGREE
        }
    });
    let metrics = measure_angular_step(
        &attitude,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: 0.0,
            target_rad: 10.0 * DEGREE,
        },
    )
    .expect("the trace is measurable");
    let settling = metrics.settling_time_s.expect("the trace settles");
    assert!(
        (settling - 1.01).abs() < 0.02,
        "settling time was {settling} s"
    );
    assert!(
        (metrics.overshoot_fraction - 0.06).abs() < 1.0e-9,
        "overshoot fraction was {}",
        metrics.overshoot_fraction
    );
    assert!((metrics.overshoot_rad - 0.6 * DEGREE).abs() < 1.0e-9);
}

#[test]
fn overshoot_is_a_fraction_of_the_requested_arc() {
    let attitude = sample_value(RATE_HZ, 3.0, |time_s| {
        if time_s < 0.5 {
            0.0
        } else if time_s < 1.0 {
            13.0 * DEGREE
        } else {
            10.0 * DEGREE
        }
    });
    let metrics = measure_angular_step(
        &attitude,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: 0.0,
            target_rad: 10.0 * DEGREE,
        },
    )
    .expect("the trace is measurable");
    assert!((metrics.overshoot_fraction - 0.3).abs() < 1.0e-9);
}

#[test]
fn a_negative_step_measures_the_same_as_its_mirror() {
    let up = first_order(0.0, 10.0 * DEGREE, 0.12, 0.5);
    let down = first_order(0.0, -10.0 * DEGREE, 0.12, 0.5);
    let spec = |target| AngularStepSpec {
        input_time_s: 0.5,
        baseline_rad: 0.0,
        target_rad: target,
    };
    let rising = measure_angular_step(&up, spec(10.0 * DEGREE)).expect("rising is measurable");
    let falling = measure_angular_step(&down, spec(-10.0 * DEGREE)).expect("falling is measurable");
    assert!((rising.amplitude_rad + falling.amplitude_rad).abs() < 1.0e-12);
    assert_eq!(rising.rise_time_s, falling.rise_time_s);
    assert_eq!(rising.settling_time_s, falling.settling_time_s);
    assert!((rising.overshoot_fraction - falling.overshoot_fraction).abs() < 1.0e-12);
}

#[test]
fn a_step_that_never_reaches_the_band_states_no_settling_time() {
    let attitude = sample_value(
        RATE_HZ,
        3.0,
        |time_s| {
            if time_s < 0.5 { 0.0 } else { 5.0 * DEGREE }
        },
    );
    let metrics = measure_angular_step(
        &attitude,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: 0.0,
            target_rad: 10.0 * DEGREE,
        },
    )
    .expect("the trace is measurable");
    assert_eq!(metrics.settling_time_s, None);
    assert_eq!(metrics.rise_time_s, None);
    assert!((metrics.steady_state_error_rad + 5.0 * DEGREE).abs() < 1.0e-9);
}

#[test]
fn a_zero_arc_request_is_refused() {
    let attitude = sample_value(RATE_HZ, 2.0, |_| 0.0);
    let error = measure_angular_step(
        &attitude,
        AngularStepSpec {
            input_time_s: 0.5,
            baseline_rad: 0.0,
            // A full turn is the same angle, so this asks for nothing.
            target_rad: 2.0 * PI,
        },
    )
    .expect_err("a zero arc has no response");
    assert!(matches!(error, MetricError::ZeroStep));
}

#[test]
fn the_short_arc_reduction_is_single_valued_at_the_boundary() {
    assert!((shortest_arc_rad(PI) - PI).abs() < 1.0e-12);
    assert!((shortest_arc_rad(-PI) - PI).abs() < 1.0e-12);
    assert!((shortest_arc_rad(3.0 * PI) - PI).abs() < 1.0e-12);
    assert!(shortest_arc_rad(0.0).abs() < 1.0e-12);
    for degrees in [-359.0, -181.0, -1.0, 1.0, 181.0, 359.0, 721.0] {
        let reduced = shortest_arc_rad(degrees * DEGREE);
        assert!(
            reduced > -PI - 1.0e-12 && reduced <= PI + 1.0e-12,
            "{degrees} degrees reduced to {reduced} rad"
        );
    }
}

#[test]
fn an_input_event_outside_the_series_is_refused() {
    let attitude = first_order(0.0, 10.0 * DEGREE, 0.1, 0.5);
    let error = measure_angular_step(
        &attitude,
        AngularStepSpec {
            input_time_s: 40.0,
            baseline_rad: 0.0,
            target_rad: 10.0 * DEGREE,
        },
    )
    .expect_err("an event outside the series has no window");
    assert!(matches!(error, MetricError::EventOutsideSeries { .. }));
}

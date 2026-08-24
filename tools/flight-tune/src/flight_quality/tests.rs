#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use crate::{GateEvaluator, MetricEvaluator};

use super::{
    CanonicalTelemetryKey, FlightQualityGate, FlightQualityGateConfig, FlightQualityGateEvaluator,
    FlightQualityMetricConfig, FlightQualityMetricEvaluator, FlightQualityScales,
    FlightQualityScenario, FlightQualityWeights, ReleasePlan, StepPlan, WindPlan,
};
use crate::{Digest, ScenarioRef, TelemetrySample};

#[test]
fn the_first_hard_gate_failure_has_priority() {
    let config = gate_config(vec![
        FlightQualityGate::CrashOrUnexpectedContact,
        FlightQualityGate::FiniteSignals,
        FlightQualityGate::EstimatorValidity,
    ]);
    let mut evaluator = FlightQualityGateEvaluator::new(config).expect("gate evaluator");
    evaluator.begin(&scenario("quality")).expect("begin gates");
    let sample = TelemetrySample {
        sequence: 0,
        elapsed_ms: 0,
        values: BTreeMap::from([(
            CanonicalTelemetryKey::CrashDetected.as_str().to_owned(),
            1.0,
        )]),
    };

    let outcomes = evaluator.evaluate(&sample).expect("evaluate gates");

    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].id,
        FlightQualityGate::CrashOrUnexpectedContact.id()
    );
    assert!(!outcomes[0].passed);
}

#[test]
fn a_missing_canonical_signal_fails_closed() {
    let config = gate_config(vec![FlightQualityGate::FiniteSignals]);
    let mut evaluator = FlightQualityGateEvaluator::new(config).expect("gate evaluator");
    evaluator.begin(&scenario("quality")).expect("begin gates");
    let mut sample = healthy_sample(0, 0, TraceValues::default());
    sample
        .values
        .remove(CanonicalTelemetryKey::WindPositionErrorM.as_str());

    let outcomes = evaluator.evaluate(&sample).expect("evaluate gates");

    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].passed);
    assert!(outcomes[0].detail.contains("has no canonical field"));
}

#[test]
fn the_metric_collector_rejects_a_missing_canonical_signal() {
    let plan = FlightQualityScenario {
        step: None,
        release: None,
        wind: None,
    };
    let mut evaluator = metric_evaluator("missing", plan, weights(0.0, 0.0, 1.0, 0.0));
    evaluator.begin(&scenario("missing")).expect("begin metric");
    let mut sample = healthy_sample(0, 0, TraceValues::default());
    sample
        .values
        .remove(CanonicalTelemetryKey::AccelerationPrimaryMps2.as_str());

    assert!(evaluator.observe(&sample).is_err());
}

#[test]
fn continuous_saturation_fails_at_the_configured_duration() {
    let config = gate_config(vec![FlightQualityGate::ActuatorSaturationDuration]);
    let mut evaluator = FlightQualityGateEvaluator::new(config).expect("gate evaluator");
    evaluator.begin(&scenario("quality")).expect("begin gates");
    for (sequence, elapsed_ms) in [0_u64, 300, 600].into_iter().enumerate() {
        let sample = healthy_sample(
            sequence as u64,
            elapsed_ms,
            TraceValues {
                saturated: true,
                ..TraceValues::default()
            },
        );
        let outcomes = evaluator.evaluate(&sample).expect("evaluate gates");
        if elapsed_ms < 600 {
            assert!(outcomes[0].passed);
        } else {
            assert!(!outcomes[0].passed);
        }
    }
}

#[test]
fn release_metrics_detect_rebound_and_zero_crossings() {
    let plan = FlightQualityScenario {
        step: Some(StepPlan {
            input_time_s: 0.2,
            initial_value: 0.0,
            target_value: 1.0,
        }),
        release: Some(ReleasePlan {
            release_time_s: 0.4,
            hold_start_s: 1.2,
            hold_position_m: 1.0,
        }),
        wind: None,
    };
    let mut evaluator = metric_evaluator("rebound", plan, weights(1.0, 1.0, 1.0, 0.0));
    evaluator.begin(&scenario("rebound")).expect("begin metric");
    let trace = [
        trace(0, 0.0, 1.0, 0.0, 0.0),
        trace(200, 0.2, 1.0, 0.0, 0.0),
        trace(400, 0.4, 1.0, 1.0, 0.2),
        trace(600, 0.58, 0.5, 0.0, 0.9),
        trace(800, 0.65, 0.0, -1.0, 1.1),
        trace(1_000, 0.75, 0.0, 0.0, 1.0),
        trace(1_200, 1.10, 0.0, 0.5, 1.0),
        trace(1_400, 0.90, 0.0, -0.5, 1.0),
        trace(1_600, 1.08, 0.0, 0.5, 1.0),
        trace(1_800, 0.98, 0.0, -0.5, 1.0),
        trace(2_000, 1.00, 0.0, 0.0, 1.0),
    ];
    observe_trace(&mut evaluator, &trace);

    evaluator.finish().expect("finish metric");
    let report = evaluator.last_report().expect("quality report");
    let hold = report.hold.expect("hold metrics");

    assert!(hold.rebound_distance_m >= 0.08);
    assert!(hold.zero_crossings >= 3);
    assert!(report.step_response.expect("step metrics").overshoot > 0.0);
    assert!(report.jerk.peak_jerk_mps3 > 0.0);
    assert!(report.control.effort_rms > 0.0);
}

#[test]
fn wind_position_offset_increases_the_loss() {
    let plan = FlightQualityScenario {
        step: None,
        release: None,
        wind: Some(WindPlan {
            minimum_wind_speed_mps: 1.0,
        }),
    };
    let weights = weights(0.0, 0.0, 0.0, 1.0);
    let low = wind_loss("wind-low", plan.clone(), weights, 0.1);
    let high = wind_loss("wind-high", plan, weights, 0.5);

    assert!(high > low * 4.9);
}

#[test]
fn evaluator_identity_changes_with_configuration() {
    let plan = FlightQualityScenario {
        step: None,
        release: None,
        wind: None,
    };
    let first = metric_evaluator("identity", plan.clone(), weights(0.0, 0.0, 1.0, 0.0));
    let mut config = metric_config("identity", plan, weights(0.0, 0.0, 1.0, 0.0));
    config.scales.jerk_mps3 = 2.0;
    let second = FlightQualityMetricEvaluator::new(config).expect("second evaluator");

    assert_ne!(first.identity().digest, second.identity().digest);
}

#[derive(Clone, Copy)]
struct TraceValues {
    position_m: f64,
    velocity_mps: f64,
    acceleration_mps2: f64,
    response: f64,
    wind_speed_mps: f64,
    wind_error_m: f64,
    saturated: bool,
}

impl Default for TraceValues {
    fn default() -> Self {
        Self {
            position_m: 0.0,
            velocity_mps: 0.0,
            acceleration_mps2: 0.0,
            response: 0.0,
            wind_speed_mps: 0.0,
            wind_error_m: 0.0,
            saturated: false,
        }
    }
}

fn trace(
    elapsed_ms: u64,
    position_m: f64,
    velocity_mps: f64,
    acceleration_mps2: f64,
    response: f64,
) -> (u64, TraceValues) {
    (
        elapsed_ms,
        TraceValues {
            position_m,
            velocity_mps,
            acceleration_mps2,
            response,
            ..TraceValues::default()
        },
    )
}

fn observe_trace(evaluator: &mut FlightQualityMetricEvaluator, trace: &[(u64, TraceValues)]) {
    for (sequence, (elapsed_ms, values)) in trace.iter().copied().enumerate() {
        evaluator
            .observe(&healthy_sample(sequence as u64, elapsed_ms, values))
            .expect("observe metric");
    }
}

fn wind_loss(
    scenario_id: &str,
    plan: FlightQualityScenario,
    weights: FlightQualityWeights,
    error_m: f64,
) -> f64 {
    let mut evaluator = metric_evaluator(scenario_id, plan, weights);
    evaluator
        .begin(&scenario(scenario_id))
        .expect("begin wind metric");
    let trace = [0_u64, 100, 200, 300].map(|elapsed_ms| {
        (
            elapsed_ms,
            TraceValues {
                wind_speed_mps: 4.0,
                wind_error_m: error_m,
                ..TraceValues::default()
            },
        )
    });
    observe_trace(&mut evaluator, &trace);
    evaluator.finish().expect("finish wind metric").loss
}

fn healthy_sample(sequence: u64, elapsed_ms: u64, values: TraceValues) -> TelemetrySample {
    let command = if elapsed_ms >= 200 { 1.0 } else { 0.0 };
    let fields = [
        (CanonicalTelemetryKey::CrashDetected, 0.0),
        (CanonicalTelemetryKey::UnexpectedContact, 0.0),
        (CanonicalTelemetryKey::PositionErrorM, 0.0),
        (CanonicalTelemetryKey::AttitudeErrorRad, 0.0),
        (CanonicalTelemetryKey::BodyRateRadS, 0.0),
        (CanonicalTelemetryKey::LoadFactorG, 1.0),
        (CanonicalTelemetryKey::ActuatorEffort, 0.2),
        (
            CanonicalTelemetryKey::ActuatorSaturated,
            if values.saturated { 1.0 } else { 0.0 },
        ),
        (CanonicalTelemetryKey::EstimatorValid, 1.0),
        (CanonicalTelemetryKey::CommandLinkValid, 1.0),
        (CanonicalTelemetryKey::Recovered, 1.0),
        (CanonicalTelemetryKey::CommandPrimary, command),
        (CanonicalTelemetryKey::ResponsePrimary, values.response),
        (CanonicalTelemetryKey::PositionPrimaryM, values.position_m),
        (
            CanonicalTelemetryKey::VelocityPrimaryMps,
            values.velocity_mps,
        ),
        (
            CanonicalTelemetryKey::AccelerationPrimaryMps2,
            values.acceleration_mps2,
        ),
        (CanonicalTelemetryKey::WindSpeedMps, values.wind_speed_mps),
        (
            CanonicalTelemetryKey::WindPositionErrorM,
            values.wind_error_m,
        ),
    ];
    TelemetrySample {
        sequence,
        elapsed_ms,
        values: fields
            .into_iter()
            .map(|(key, value)| (key.as_str().to_owned(), value))
            .collect(),
    }
}

fn gate_config(required: Vec<FlightQualityGate>) -> FlightQualityGateConfig {
    FlightQualityGateConfig {
        required,
        maximum_position_error_m: 5.0,
        maximum_attitude_error_rad: 1.0,
        maximum_body_rate_rad_s: 5.0,
        maximum_load_factor_g: 4.0,
        maximum_saturation_s: 0.5,
        recovery_deadline_s: 2.0,
    }
}

fn metric_evaluator(
    scenario_id: &str,
    plan: FlightQualityScenario,
    weights: FlightQualityWeights,
) -> FlightQualityMetricEvaluator {
    FlightQualityMetricEvaluator::new(metric_config(scenario_id, plan, weights))
        .expect("metric evaluator")
}

fn metric_config(
    scenario_id: &str,
    plan: FlightQualityScenario,
    weights: FlightQualityWeights,
) -> FlightQualityMetricConfig {
    FlightQualityMetricConfig {
        scenarios: BTreeMap::from([(scenario_id.to_owned(), plan)]),
        scales: FlightQualityScales {
            time_s: 1.0,
            position_m: 1.0,
            speed_mps: 1.0,
            jerk_mps3: 10.0,
            unreached_penalty: 5.0,
        },
        weights,
    }
}

const fn weights(
    step_response: f64,
    release: f64,
    jerk: f64,
    wind_position: f64,
) -> FlightQualityWeights {
    FlightQualityWeights {
        step_response,
        release,
        jerk,
        wind_position,
    }
}

fn scenario(id: &str) -> ScenarioRef {
    ScenarioRef {
        id: id.to_owned(),
        digest: Digest::from_bytes([42; 32]),
        max_samples: 128,
        sample_timeout_ms: 100,
    }
}

use pilotage_flight_quality::{
    ControlMetrics, ControlPoint, HoldMetrics, JerkMetrics, MotionPoint, ReleaseMetrics,
    ResponseMetrics, SignalStats, StepSpec, TimedValue, measure_control, measure_hold,
    measure_jerk, measure_release, measure_signal, measure_step_response,
};

use crate::flight_quality::config::{
    FlightQualityMetricConfig, FlightQualityScales, FlightQualityScenario, ReleasePlan, StepPlan,
    WindPlan, metric_identity,
};
use crate::flight_quality::telemetry::CanonicalSample;
use crate::{
    ArtifactIdentity, EvaluatorError, MetricEvaluator, MetricValues, MissionReference,
    TelemetrySample,
};

/// Detailed metrics for the last completed scenario run.
#[derive(Debug, Clone, PartialEq)]
pub struct FlightQualityReport {
    /// Step-response metrics when the scenario has a step plan.
    pub step_response: Option<ResponseMetrics>,
    /// Release metrics when the scenario has a release plan.
    pub release: Option<ReleaseMetrics>,
    /// Final-hold metrics when the scenario has a release plan.
    pub hold: Option<HoldMetrics>,
    /// Acceleration and jerk metrics.
    pub jerk: JerkMetrics,
    /// Control-effort and saturation metrics.
    pub control: ControlMetrics,
    /// Wind-position statistics when the scenario has a wind plan.
    pub wind_position: Option<SignalStats>,
    /// The normalized weighted loss supplied to the tuner.
    pub dimensionless_loss: f64,
}

/// A streaming collector that uses `pilotage-flight-quality` algorithms.
pub struct FlightQualityMetricEvaluator {
    config: FlightQualityMetricConfig,
    identity: ArtifactIdentity,
    active: Option<ActiveRun>,
    last_report: Option<FlightQualityReport>,
}

struct ActiveRun {
    plan: FlightQualityScenario,
    samples: Vec<CanonicalSample>,
}

impl FlightQualityMetricEvaluator {
    /// Creates an evaluator and hashes its configuration and implementation.
    ///
    /// # Errors
    ///
    /// Returns [`EvaluatorError`] when the configuration is not valid.
    pub fn new(config: FlightQualityMetricConfig) -> Result<Self, EvaluatorError> {
        config.validate()?;
        let identity = metric_identity(&config)?;
        Ok(Self {
            config,
            identity,
            active: None,
            last_report: None,
        })
    }

    /// Returns the report from the last successful run.
    #[must_use]
    pub const fn last_report(&self) -> Option<&FlightQualityReport> {
        self.last_report.as_ref()
    }
}

impl MetricEvaluator for FlightQualityMetricEvaluator {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, scenario: &MissionReference) -> Result<(), EvaluatorError> {
        if self.active.is_some() {
            return Err(invalid("a metric run is already active"));
        }
        let plan = self
            .config
            .scenarios
            .get(&scenario.revision_id)
            .cloned()
            .ok_or_else(|| {
                invalid(format!(
                    "scenario {} has no metric plan",
                    scenario.revision_id
                ))
            })?;
        self.last_report = None;
        self.active = Some(ActiveRun {
            plan,
            samples: Vec::new(),
        });
        Ok(())
    }

    fn observe(&mut self, sample: &TelemetrySample) -> Result<(), EvaluatorError> {
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| invalid("no metric run is active"))?;
        let decoded = CanonicalSample::decode(sample)?;
        validate_sample(active.samples.last(), &decoded)?;
        active.samples.push(decoded);
        Ok(())
    }

    fn finish(&mut self) -> Result<MetricValues, EvaluatorError> {
        let active = self
            .active
            .take()
            .ok_or_else(|| invalid("no metric run is active"))?;
        let report = build_report(&active, &self.config)?;
        let values = MetricValues {
            loss: report.dimensionless_loss,
            control_effort: report.control.effort_rms.clamp(0.0, 1.0),
            objectives: objective_values(&report),
        };
        self.last_report = Some(report);
        Ok(values)
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        self.active = None;
        Ok(())
    }
}

fn objective_values(report: &FlightQualityReport) -> BTreeMap<String, f64> {
    let mut values = BTreeMap::from([
        (
            "jerk.peak_acceleration_mps2".to_owned(),
            report.jerk.peak_acceleration_mps2,
        ),
        ("jerk.peak_mps3".to_owned(), report.jerk.peak_jerk_mps3),
        ("jerk.p95_mps3".to_owned(), report.jerk.jerk_p95_mps3),
        ("jerk.rms_mps3".to_owned(), report.jerk.jerk_rms_mps3),
        ("control.effort_rms".to_owned(), report.control.effort_rms),
        (
            "control.saturation_fraction".to_owned(),
            report.control.saturation_fraction,
        ),
        (
            "control.longest_saturation_s".to_owned(),
            report.control.longest_saturation_s,
        ),
    ]);
    insert_step_objectives(&mut values, report.step_response);
    insert_release_objectives(&mut values, report.release);
    insert_hold_objectives(&mut values, report.hold);
    insert_wind_objectives(&mut values, report.wind_position);
    values
}

fn insert_step_objectives(values: &mut BTreeMap<String, f64>, step: Option<ResponseMetrics>) {
    let Some(step) = step else { return };
    values.extend([
        (
            "step.command_delay_s".to_owned(),
            required(step.input_to_command_delay_s),
        ),
        (
            "step.response_delay_s".to_owned(),
            required(step.input_to_response_delay_s),
        ),
        ("step.rise_time_s".to_owned(), required(step.rise_time_s)),
        (
            "step.settling_time_s".to_owned(),
            required(step.settling_time_s),
        ),
        (
            "step.overshoot_fraction".to_owned(),
            step.overshoot_fraction,
        ),
        ("step.undershoot".to_owned(), step.undershoot),
    ]);
}

fn insert_release_objectives(values: &mut BTreeMap<String, f64>, release: Option<ReleaseMetrics>) {
    let Some(release) = release else { return };
    values.extend([
        (
            "release.stop_time_s".to_owned(),
            required(release.release_to_stop_s),
        ),
        (
            "release.brake_distance_m".to_owned(),
            required(release.brake_distance_m),
        ),
        (
            "release.return_toward_release_m".to_owned(),
            release.return_toward_release_m,
        ),
        (
            "release.opposite_velocity_peak_mps".to_owned(),
            release.opposite_velocity_peak_mps,
        ),
    ]);
}

fn insert_hold_objectives(values: &mut BTreeMap<String, f64>, hold: Option<HoldMetrics>) {
    let Some(hold) = hold else { return };
    values.extend([
        (
            "hold.rebound_distance_m".to_owned(),
            hold.rebound_distance_m,
        ),
        (
            "hold.zero_crossings".to_owned(),
            f64::from(hold.zero_crossings),
        ),
    ]);
}

fn insert_wind_objectives(values: &mut BTreeMap<String, f64>, wind: Option<SignalStats>) {
    let Some(wind) = wind else { return };
    values.extend([
        ("wind.position_rms_m".to_owned(), wind.rms),
        ("wind.position_p95_m".to_owned(), wind.p95_abs),
        ("wind.position_peak_m".to_owned(), wind.peak_abs),
    ]);
}

fn required(value: Option<f64>) -> f64 {
    value.unwrap_or(f64::MAX)
}

fn validate_sample(
    prior: Option<&CanonicalSample>,
    sample: &CanonicalSample,
) -> Result<(), EvaluatorError> {
    if prior.is_some_and(|prior| sample.time_s <= prior.time_s) {
        return Err(invalid("canonical sample time must increase"));
    }
    if !(-1.0..=1.0).contains(&sample.effort)
        || sample.wind_speed_mps < 0.0
        || sample.wind_position_error_m < 0.0
    {
        return Err(invalid(
            "canonical effort or wind value is outside its domain",
        ));
    }
    Ok(())
}

fn build_report(
    active: &ActiveRun,
    config: &FlightQualityMetricConfig,
) -> Result<FlightQualityReport, EvaluatorError> {
    if active.samples.len() < 2 {
        return Err(invalid("a flight-quality metric needs two samples"));
    }
    let command = timed_values(&active.samples, |sample| sample.command);
    let response = timed_values(&active.samples, |sample| sample.response);
    let position = timed_values(&active.samples, |sample| sample.position_m);
    let acceleration = timed_values(&active.samples, |sample| sample.acceleration_mps2);
    let motion = motion_points(&active.samples);
    let control = measure_control(&control_points(&active.samples)).map_err(metric_error)?;
    let jerk = measure_jerk(&acceleration).map_err(metric_error)?;
    let step = active
        .plan
        .step
        .map(|plan| measure_step(&command, &response, plan))
        .transpose()?;
    let release = active
        .plan
        .release
        .map(|plan| measure_release_metrics(&motion, &position, plan))
        .transpose()?;
    let wind = active
        .plan
        .wind
        .map(|plan| measure_wind(&active.samples, plan))
        .transpose()?;
    let (release_metrics, hold_metrics) = release.unzip();
    let loss = loss(
        step.as_ref(),
        release_metrics.as_ref(),
        hold_metrics.as_ref(),
        &jerk,
        wind.as_ref(),
        active,
        config,
    )?;
    Ok(FlightQualityReport {
        step_response: step,
        release: release_metrics,
        hold: hold_metrics,
        jerk,
        control,
        wind_position: wind,
        dimensionless_loss: loss,
    })
}

fn measure_step(
    command: &[TimedValue],
    response: &[TimedValue],
    plan: StepPlan,
) -> Result<ResponseMetrics, EvaluatorError> {
    measure_step_response(
        command,
        response,
        StepSpec {
            input_time_s: plan.input_time_s,
            initial_value: plan.initial_value,
            target_value: plan.target_value,
        },
    )
    .map_err(metric_error)
}

fn measure_release_metrics(
    motion: &[MotionPoint],
    position: &[TimedValue],
    plan: ReleasePlan,
) -> Result<(ReleaseMetrics, HoldMetrics), EvaluatorError> {
    let release =
        measure_release(motion, plan.release_time_s, plan.hold_start_s).map_err(metric_error)?;
    let hold =
        measure_hold(position, plan.hold_start_s, plan.hold_position_m).map_err(metric_error)?;
    Ok((release, hold))
}

fn measure_wind(
    samples: &[CanonicalSample],
    plan: WindPlan,
) -> Result<SignalStats, EvaluatorError> {
    let first = samples
        .iter()
        .position(|sample| sample.wind_speed_mps >= plan.minimum_wind_speed_mps)
        .ok_or_else(|| invalid("the trace has no active wind interval"))?;
    let last = samples
        .iter()
        .rposition(|sample| sample.wind_speed_mps >= plan.minimum_wind_speed_mps)
        .ok_or_else(|| invalid("the trace has no active wind interval"))?;
    let wind = samples[first..=last]
        .iter()
        .map(|sample| TimedValue {
            time_s: sample.time_s,
            value: sample.wind_position_error_m,
        })
        .collect::<Vec<_>>();
    measure_signal(&wind).map_err(metric_error)
}

fn timed_values(
    samples: &[CanonicalSample],
    value: impl Fn(&CanonicalSample) -> f64,
) -> Vec<TimedValue> {
    samples
        .iter()
        .map(|sample| TimedValue {
            time_s: sample.time_s,
            value: value(sample),
        })
        .collect()
}

fn motion_points(samples: &[CanonicalSample]) -> Vec<MotionPoint> {
    samples
        .iter()
        .map(|sample| MotionPoint {
            time_s: sample.time_s,
            position_m: sample.position_m,
            velocity_mps: sample.velocity_mps,
        })
        .collect()
}

fn control_points(samples: &[CanonicalSample]) -> Vec<ControlPoint> {
    samples
        .iter()
        .map(|sample| ControlPoint {
            time_s: sample.time_s,
            effort: sample.effort,
            saturated: sample.saturated,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn loss(
    step: Option<&ResponseMetrics>,
    release: Option<&ReleaseMetrics>,
    hold: Option<&HoldMetrics>,
    jerk: &JerkMetrics,
    wind: Option<&SignalStats>,
    active: &ActiveRun,
    config: &FlightQualityMetricConfig,
) -> Result<f64, EvaluatorError> {
    let scales = config.scales;
    let weights = config.weights;
    let mut weighted = weights.jerk * jerk_component(jerk, scales);
    let mut total_weight = weights.jerk;
    if let (Some(metrics), Some(plan)) = (step, active.plan.step) {
        weighted += weights.step_response * step_component(metrics, plan, scales);
        total_weight += weights.step_response;
    }
    if let (Some(release), Some(hold)) = (release, hold) {
        weighted += weights.release * release_component(release, hold, scales);
        total_weight += weights.release;
    }
    if let Some(metrics) = wind {
        weighted += weights.wind_position * wind_component(metrics, scales);
        total_weight += weights.wind_position;
    }
    if total_weight <= 0.0 || active.samples.is_empty() {
        return Err(invalid("the active scenario has no positive metric weight"));
    }
    let value = weighted / total_weight;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(invalid("flight-quality loss is not finite and nonnegative"))
    }
}

fn step_component(metrics: &ResponseMetrics, plan: StepPlan, scales: FlightQualityScales) -> f64 {
    let time = |value: Option<f64>| value.map_or(scales.unreached_penalty, |v| v / scales.time_s);
    let amplitude = (plan.target_value - plan.initial_value).abs();
    (time(metrics.input_to_command_delay_s)
        + time(metrics.input_to_response_delay_s)
        + time(metrics.rise_time_s)
        + time(metrics.settling_time_s)
        + metrics.overshoot_fraction
        + metrics.undershoot / amplitude)
        / 6.0
}

fn release_component(
    release: &ReleaseMetrics,
    hold: &HoldMetrics,
    scales: FlightQualityScales,
) -> f64 {
    let stop = release
        .release_to_stop_s
        .map_or(scales.unreached_penalty, |value| value / scales.time_s);
    let brake = release
        .brake_distance_m
        .map_or(scales.unreached_penalty, |value| value / scales.position_m);
    (stop
        + brake
        + release.return_toward_release_m / scales.position_m
        + release.opposite_velocity_peak_mps / scales.speed_mps
        + hold.rebound_distance_m / scales.position_m
        + f64::from(hold.zero_crossings))
        / 6.0
}

fn jerk_component(metrics: &JerkMetrics, scales: FlightQualityScales) -> f64 {
    (metrics.peak_jerk_mps3 + metrics.jerk_p95_mps3 + metrics.jerk_rms_mps3)
        / (3.0 * scales.jerk_mps3)
}

fn wind_component(metrics: &SignalStats, scales: FlightQualityScales) -> f64 {
    (metrics.rms + metrics.p95_abs + metrics.peak_abs) / (3.0 * scales.position_m)
}

fn metric_error(error: pilotage_flight_quality::MetricError) -> EvaluatorError {
    invalid(format!("pilotage flight-quality metric failed: {error}"))
}

fn invalid(detail: impl Into<String>) -> EvaluatorError {
    EvaluatorError::new(detail)
}
use std::collections::BTreeMap;

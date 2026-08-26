//! Turning one trial's telemetry into the objectives a bar is stated over.
//!
//! The measurements in `pilotage-flight-quality` return typed results; a
//! campaign needs them as the named values final qualification looks for. This
//! is that bridge, and it is the layer both vehicles share: a new aircraft
//! contributes limits, not a metric.
//!
//! SIM / NOT FOR FLIGHT.

use std::collections::BTreeMap;

use flight_tune::{
    ArtifactIdentity, Digest, EvaluatorError, MetricEvaluator, MetricValues, ScenarioRef,
    TelemetrySample,
};
use pilotage_flight_quality::{
    ControlPoint, MotionPoint, StepSpec, TimedValue, measure_control, measure_hold, measure_jerk,
    measure_release, measure_step_response,
};

/// The channels a backend supplies for one sample.
///
/// The set is deliberately small and vehicle-neutral: a position, a velocity
/// and an acceleration on the axis under test, the demand that produced them,
/// the response that demand asked for, and which phase of the trial the run is
/// in. A simulator that can report those can be tuned; nothing here knows what
/// kind of aircraft it is.
pub mod channel {
    /// Normalized demand on the axis under test.
    pub const COMMAND: &str = "command";
    /// Normalized response on the same axis, in the same units as the demand.
    pub const RESPONSE: &str = "response";
    /// Position on the axis under test, in metres.
    pub const POSITION_M: &str = "position_m";
    /// Velocity on the axis under test, in metres per second.
    pub const VELOCITY_MPS: &str = "velocity_mps";
    /// Acceleration on the axis under test, in metres per second squared.
    pub const ACCELERATION_MPS2: &str = "acceleration_mps2";
    /// Normalized actuator demand.
    pub const EFFORT: &str = "effort";
    /// Nonzero while an actuator limit is active.
    pub const SATURATED: &str = "saturated";
    /// Which phase the trial is in: 0 idle, 1 step, 2 hold, 3 release,
    /// 4 settled.
    pub const PHASE: &str = "phase";
}

/// Trial phases, read from the run rather than assumed of it.
///
/// The evaluator takes the transition times from what the backend reported,
/// not from what a scenario said should happen. A run whose step arrived late
/// is measured from when it arrived; a run that never released is a run whose
/// release metrics cannot be computed, and saying so is better than measuring
/// a release that did not occur.
const PHASE_IDLE: f64 = 0.0;
const PHASE_STEP: f64 = 1.0;
const PHASE_HOLD: f64 = 2.0;
const PHASE_RELEASE: f64 = 3.0;
/// Where the vehicle came to rest after the release.
///
/// This is the hold the release metrics are measured against, and it is not
/// the commanded hold before it: the question a brake metric answers is where
/// the vehicle stopped once the stick was let go, which is only known after it
/// has stopped.
const PHASE_SETTLED: f64 = 4.0;

/// One trial's samples, kept until the run completes.
#[derive(Debug, Default)]
struct Trace {
    command: Vec<TimedValue>,
    response: Vec<TimedValue>,
    position: Vec<TimedValue>,
    acceleration: Vec<TimedValue>,
    motion: Vec<MotionPoint>,
    control: Vec<ControlPoint>,
    phase: Vec<TimedValue>,
}

impl Trace {
    /// The first time the run entered this phase.
    fn entered(&self, phase: f64) -> Option<f64> {
        let mut previous = PHASE_IDLE;
        for sample in &self.phase {
            if previous < phase && sample.value >= phase {
                return Some(sample.time_s);
            }
            previous = sample.value;
        }
        None
    }
}

/// Scores one trial against the shared flight-quality measurements.
#[derive(Debug)]
pub struct FlightQualityEvaluator {
    identity: ArtifactIdentity,
    trace: Trace,
    active: bool,
}

impl FlightQualityEvaluator {
    /// Creates one evaluator with a stated implementation identity.
    ///
    /// # Errors
    ///
    /// Returns [`EvaluatorError`] when the identity cannot be built.
    pub fn new(identity_digest: Digest) -> Result<Self, EvaluatorError> {
        let identity = ArtifactIdentity::new("pilotage-flight-quality", identity_digest)
            .map_err(|error| EvaluatorError::new(error.to_string()))?;
        Ok(Self {
            identity,
            trace: Trace::default(),
            active: false,
        })
    }
}

impl MetricEvaluator for FlightQualityEvaluator {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, _scenario: &ScenarioRef) -> Result<(), EvaluatorError> {
        self.trace = Trace::default();
        self.active = true;
        Ok(())
    }

    fn observe(&mut self, sample: &TelemetrySample) -> Result<(), EvaluatorError> {
        if !self.active {
            return Err(EvaluatorError::new("a sample arrived before the run began"));
        }
        let time_s = f64::from(u32::try_from(sample.elapsed_ms).unwrap_or(u32::MAX)) / 1_000.0;
        let read = |name: &str| -> Result<f64, EvaluatorError> {
            sample
                .values
                .get(name)
                .copied()
                .filter(|value| value.is_finite())
                .ok_or_else(|| EvaluatorError::new(format!("the sample states no {name}")))
        };

        let position_m = read(channel::POSITION_M)?;
        let velocity_mps = read(channel::VELOCITY_MPS)?;
        self.trace.command.push(TimedValue {
            time_s,
            value: read(channel::COMMAND)?,
        });
        self.trace.response.push(TimedValue {
            time_s,
            value: read(channel::RESPONSE)?,
        });
        self.trace.position.push(TimedValue {
            time_s,
            value: position_m,
        });
        self.trace.acceleration.push(TimedValue {
            time_s,
            value: read(channel::ACCELERATION_MPS2)?,
        });
        self.trace.motion.push(MotionPoint {
            time_s,
            position_m,
            velocity_mps,
        });
        self.trace.control.push(ControlPoint {
            time_s,
            effort: read(channel::EFFORT)?,
            saturated: read(channel::SATURATED)? != 0.0,
        });
        self.trace.phase.push(TimedValue {
            time_s,
            value: read(channel::PHASE)?,
        });
        Ok(())
    }

    fn finish(&mut self) -> Result<MetricValues, EvaluatorError> {
        if !self.active {
            return Err(EvaluatorError::new("the run did not begin"));
        }
        self.active = false;
        let trace = core::mem::take(&mut self.trace);
        let fail = |error: pilotage_flight_quality::MetricError| {
            EvaluatorError::new(format!("a measurement refused this run: {error}"))
        };

        let step_at = trace
            .entered(PHASE_STEP)
            .ok_or_else(|| EvaluatorError::new("the run never left idle"))?;
        let hold_at = trace
            .entered(PHASE_HOLD)
            .ok_or_else(|| EvaluatorError::new("the run never reached its hold"))?;
        let release_at = trace
            .entered(PHASE_RELEASE)
            .ok_or_else(|| EvaluatorError::new("the run never released"))?;
        let settled_at = trace
            .entered(PHASE_SETTLED)
            .ok_or_else(|| EvaluatorError::new("the run never settled after its release"))?;

        let control = measure_control(&trace.control).map_err(fail)?;
        let jerk = measure_jerk(&trace.acceleration).map_err(fail)?;
        // The step is described by what the run did: the demand before the
        // input and the demand it settled on after it.
        let initial_value = value_at(&trace.command, step_at, true);
        let target_value = value_at(&trace.command, hold_at, false);
        // The step response is measured over the step, which ends when the
        // stick is let go. Measured over the whole trial it never settles: the
        // response leaves the band again at release, and "the final entry into
        // the band" is then a thing that never happened.
        let command_window = window(&trace.command, step_at, release_at);
        let response_window = window(&trace.response, step_at, release_at);
        let response = measure_step_response(
            &command_window,
            &response_window,
            StepSpec {
                input_time_s: step_at,
                initial_value,
                target_value,
            },
        )
        .map_err(fail)?;
        let release = measure_release(&trace.motion, release_at, settled_at).map_err(fail)?;
        let hold_position = value_at(&trace.position, settled_at, false);
        let hold = measure_hold(&trace.position, settled_at, hold_position).map_err(fail)?;

        let objectives = BTreeMap::from([
            ("control.effort_rms".to_owned(), control.effort_rms),
            (
                "control.longest_saturation_s".to_owned(),
                control.longest_saturation_s,
            ),
            (
                "control.saturation_fraction".to_owned(),
                control.saturation_fraction,
            ),
            (
                "hold.zero_crossings".to_owned(),
                f64::from(hold.zero_crossings),
            ),
            (
                "hold.rebound_distance_m".to_owned(),
                hold.rebound_distance_m.abs(),
            ),
            (
                "jerk.peak_acceleration_mps2".to_owned(),
                jerk.peak_acceleration_mps2,
            ),
            ("jerk.peak_jerk_mps3".to_owned(), jerk.peak_jerk_mps3),
            ("jerk.jerk_p95_mps3".to_owned(), jerk.jerk_p95_mps3),
            ("jerk.jerk_rms_mps3".to_owned(), jerk.jerk_rms_mps3),
            (
                "release.brake_distance_m".to_owned(),
                required(release.brake_distance_m, "release.brake_distance_m")?.abs(),
            ),
            (
                "release.opposite_velocity_peak_mps".to_owned(),
                release.opposite_velocity_peak_mps.abs(),
            ),
            (
                "release.return_toward_release_m".to_owned(),
                release.return_toward_release_m.abs(),
            ),
            (
                "release.release_to_stop_s".to_owned(),
                required(release.release_to_stop_s, "release.release_to_stop_s")?,
            ),
            (
                "response.input_to_command_delay_s".to_owned(),
                required(
                    response.input_to_command_delay_s,
                    "response.input_to_command_delay_s",
                )?,
            ),
            (
                "response.input_to_response_delay_s".to_owned(),
                required(
                    response.input_to_response_delay_s,
                    "response.input_to_response_delay_s",
                )?,
            ),
            (
                "response.overshoot_fraction".to_owned(),
                response.overshoot_fraction.abs(),
            ),
            (
                "response.rise_time_s".to_owned(),
                required(response.rise_time_s, "response.rise_time_s")?,
            ),
            (
                "response.settling_time_s".to_owned(),
                required(response.settling_time_s, "response.settling_time_s")?,
            ),
        ]);

        // The scalar the search minimises is the mean absolute tracking error
        // over the trial. It is chosen rather than a weighted sum of the
        // objectives above because a weighted sum needs the bar's ceilings to
        // normalise against, and an evaluator that read the bar it is scored
        // against could be made to agree with any bar. The objectives carry the
        // rest of the judgement, each against its own stated limit.
        let duration_s = trace
            .response
            .last()
            .zip(trace.response.first())
            .map_or(0.0, |(last, first)| last.time_s - first.time_s);
        let loss = if duration_s > 0.0 {
            response.integrated_absolute_error / duration_s
        } else {
            return Err(EvaluatorError::new("the run covered no time"));
        };

        Ok(MetricValues {
            loss: loss.abs(),
            control_effort: control.effort_rms.clamp(0.0, 1.0),
            objectives,
        })
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        self.trace = Trace::default();
        self.active = false;
        Ok(())
    }
}

/// The series value at or nearest one time.
///
/// `before` selects the last sample STRICTLY before the time rather than the
/// first at or after it, which is what distinguishes "the demand the run was
/// holding when the step arrived" from "the demand it stepped to". The sample
/// at the transition already carries the new demand — the phase and the value
/// change together — so including it would make every step zero amplitude.
fn value_at(series: &[TimedValue], time_s: f64, before: bool) -> f64 {
    if before {
        series
            .iter()
            .take_while(|sample| sample.time_s < time_s)
            .last()
            .map_or(0.0, |sample| sample.value)
    } else {
        series
            .iter()
            .find(|sample| sample.time_s >= time_s)
            .map_or(0.0, |sample| sample.value)
    }
}

/// The samples between two times, inclusive of the first and exclusive of the
/// last.
fn window(series: &[TimedValue], from_s: f64, to_s: f64) -> Vec<TimedValue> {
    series
        .iter()
        .filter(|sample| sample.time_s >= from_s && sample.time_s < to_s)
        .copied()
        .collect()
}

/// A timing a bar names must be one the run produced.
///
/// These measurements are absent when the run never reached the threshold they
/// are defined by — a response that never rose to ninety percent has no rise
/// time. Final qualification requires every named objective in every run, so
/// an absent one is reported here, where it names what was missing, rather
/// than left out to fail later on the name alone.
fn required(value: Option<f64>, name: &'static str) -> Result<f64, EvaluatorError> {
    value.ok_or_else(|| EvaluatorError::new(format!("the run produced no {name}")))
}

#[cfg(test)]
mod tests;

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
    ArtifactIdentity, Digest, EvaluatorError, MetricEvaluator, MetricValues, MissionReference,
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
    /// The physical demand the candidate resolved, in the envelope's unit.
    pub const PHYSICAL_DEMAND: &str = "physical_demand";
    /// Nonzero when the simulator detected a crash.
    pub const CRASHED: &str = "crashed";
    /// Nonzero when the simulator detected unexpected ground contact.
    pub const GROUND_CONTACT: &str = "ground_contact";
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
    demand: Vec<TimedValue>,
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

    fn begin(&mut self, _scenario: &MissionReference) -> Result<(), EvaluatorError> {
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
        self.trace.demand.push(TimedValue {
            time_s,
            value: read(channel::PHYSICAL_DEMAND)?,
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

        let (step_at, hold_at, release_at, settled_at) = phase_entries(&trace)?;

        let control = measure_control(&trace.control).map_err(fail)?;
        let jerk = measure_jerk(&trace.acceleration).map_err(fail)?;
        let response = step_response(&trace, step_at, hold_at, release_at).map_err(fail)?;
        let release = measure_release(&trace.motion, release_at, settled_at).map_err(fail)?;
        let hold_position = value_at(&trace.position, settled_at, false);
        let hold = measure_hold(&trace.position, settled_at, hold_position).map_err(fail)?;

        // A candidate that never stops or never settles is MEASURED as
        // one that never did — each absent metric scores as the worst
        // its own window allows, so a bad law fails its ceilings instead
        // of aborting the campaign that is judging it.
        let worst = worst_case(
            &trace.position,
            step_at,
            release_at,
            settled_at,
            hold_position,
        );
        let mut objectives = objectives_from(&control, &jerk, &hold, &release, &response, worst);
        // The physical target the candidate resolved for the held operator
        // input. It is read at the hold, after the demand rate limits have
        // settled, so it states what the stick was worth rather than what the
        // vehicle was passing through on the way there.
        objectives.insert(
            flight_tune::TARGET_AUTHORITY_OBJECTIVE.to_owned(),
            value_at(&trace.demand, hold_at, false).abs(),
        );

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

/// The step response over the interval the stick was held.
///
/// The step is described by what the run did: the demand before the input and
/// the demand it settled on after it. The window ends when the stick is let
/// go, because measured over the whole trial the response leaves the settling
/// band again at release, and "the final entry into the band" is then a thing
/// that never happened.
fn step_response(
    trace: &Trace,
    step_at: f64,
    hold_at: f64,
    release_at: f64,
) -> Result<pilotage_flight_quality::ResponseMetrics, pilotage_flight_quality::MetricError> {
    measure_step_response(
        &window(&trace.command, step_at, release_at),
        &window(&trace.response, step_at, release_at),
        StepSpec {
            input_time_s: step_at,
            initial_value: value_at(&trace.command, step_at, true),
            target_value: value_at(&trace.command, hold_at, false),
        },
    )
}

/// The series value at or nearest one time.
///
/// `before` selects the last sample STRICTLY before the time rather than the
/// first at or after it, which is what distinguishes "the demand the run was
/// holding when the step arrived" from "the demand it stepped to". The sample
/// at the transition already carries the new demand — the phase and the value
/// change together — so including it would make every step zero amplitude.
/// One named objective per measured limit the bar states.
fn objectives_from(
    control: &pilotage_flight_quality::ControlMetrics,
    jerk: &pilotage_flight_quality::JerkMetrics,
    hold: &pilotage_flight_quality::HoldMetrics,
    release: &pilotage_flight_quality::ReleaseMetrics,
    response: &pilotage_flight_quality::ResponseMetrics,
    worst: WorstCase,
) -> BTreeMap<String, f64> {
    BTreeMap::from([
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
            release.brake_distance_m.map_or(worst.brake_m, f64::abs),
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
            release.release_to_stop_s.unwrap_or(worst.release_s),
        ),
        (
            "response.input_to_command_delay_s".to_owned(),
            response.input_to_command_delay_s.unwrap_or(worst.step_s),
        ),
        (
            "response.input_to_response_delay_s".to_owned(),
            response.input_to_response_delay_s.unwrap_or(worst.step_s),
        ),
        (
            "response.overshoot_fraction".to_owned(),
            response.overshoot_fraction.abs(),
        ),
        (
            "response.rise_time_s".to_owned(),
            response.rise_time_s.unwrap_or(worst.step_s),
        ),
        (
            "response.settling_time_s".to_owned(),
            response.settling_time_s.unwrap_or(worst.step_s),
        ),
    ])
}

/// The four phase boundaries the trial promises. Their absence is a
/// harness fault — the trial script drives the phases, not the
/// candidate — so it refuses rather than scores.
fn phase_entries(trace: &Trace) -> Result<(f64, f64, f64, f64), EvaluatorError> {
    let entered = |phase: f64, missing: &'static str| {
        trace
            .entered(phase)
            .ok_or_else(|| EvaluatorError::new(missing))
    };
    Ok((
        entered(PHASE_STEP, "the run never left idle")?,
        entered(PHASE_HOLD, "the run never reached its hold")?,
        entered(PHASE_RELEASE, "the run never released")?,
        entered(PHASE_SETTLED, "the run never settled after its release")?,
    ))
}

fn worst_case(
    position: &[TimedValue],
    step_at: f64,
    release_at: f64,
    settled_at: f64,
    hold_position: f64,
) -> WorstCase {
    let brake_m = window(position, release_at, settled_at)
        .iter()
        .map(|point| (point.value - hold_position).abs())
        .fold(0.0_f64, f64::max);
    WorstCase {
        release_s: settled_at - release_at,
        step_s: release_at - step_at,
        brake_m,
    }
}

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
/// The value each absent metric scores as: the worst its own window
/// allows. A run that never stopped is scored as stopping at the
/// window's end; one that never rose as rising at the step window's
/// end; a brake distance with no stop as the farthest the vehicle got
/// from its hold. Ceilings then judge the law; absence never aborts
/// the campaign.
#[derive(Clone, Copy)]
struct WorstCase {
    release_s: f64,
    step_s: f64,
    brake_m: f64,
}

#[cfg(test)]
mod tests;

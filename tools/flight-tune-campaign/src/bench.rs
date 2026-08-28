//! A closed loop small enough to run, over the code that actually ships.
//!
//! The shaping under test is the real [`AxisDemandShaper`] from
//! `pilotage-control-feel` — the same law the adapter installs on a vehicle.
//! What is reduced is the aircraft: a first-order velocity response with a
//! stated time constant, not a simulator.
//!
//! That trade is deliberate. A campaign proves nothing until it runs end to
//! end once, and every layer above this one — the search, the journal, the
//! promotion decision, the evidence chain, the final bar — is exercised the
//! same way whether the plant is four lines or a flight dynamics model. When a
//! simulator-backed backend arrives it replaces this one and nothing above it
//! changes, which is the property the whole contract exists for.
//!
//! Results from this backend describe the command law, not the aircraft. They
//! are not a qualified calibration for any vehicle.
//!
//! The held-out scenarios are held out in NAME only here: the plant is
//! deterministic and the trial script fixed, so promotion and final runs
//! replay the training trial under different labels. The stage still
//! enforces id and digest disjointness — what a simulator-backed backend
//! makes physically distinct, this bench keeps structurally distinct.
//!
//! SIM / NOT FOR FLIGHT.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use flight_tune::{
    AdapterError, ArtifactIdentity, Candidate, Digest, RunExecutionContext, RunPreparationReceipt,
    SampleEvent, ScenarioRef, ScenarioStartReceipt, SessionChallenge, SimulatorBackend,
    SimulatorCapability, SimulatorSessionReceipt, TelemetrySample,
};
use pilotage_control_feel::{AxisCurve, AxisDemandShaper, AxisDynamics, AxisResponse, NeutralBand};

use crate::scoring::channel;

/// The trial the bench flies: idle, step, hold, release, settle.
///
/// The phase boundaries are wall-clock positions in the trial rather than
/// conditions on the vehicle, so every run of a scenario covers the same
/// window whatever the candidate does inside it. A candidate that never
/// settles is measured as one that never settled.
const STEP_AT_S: f64 = 0.5;
const HOLD_AT_S: f64 = 3.0;
const RELEASE_AT_S: f64 = 5.0;
/// The release window has to be long enough for the heaviest vehicle to reach
/// a confirmed stop, which the brake measurement defines as staying under
/// 0.05 m/s for a fifth of a second. A first-order vehicle decaying from three
/// metres per second with a time constant of just under half a second needs
/// most of two seconds to get there, so a window that merely looked generous
/// would measure the slower aircraft as never having stopped.
const SETTLE_AT_S: f64 = 8.5;
const END_S: f64 = 10.5;
/// The bench reports at the rate a control loop runs at.
const DT_S: f64 = 0.02;

/// The parameters a candidate may set, and what they mean.
///
/// These are the shaping numbers themselves, so a search over them is a search
/// over the command law rather than over a proxy for it.
pub mod parameter {
    /// Input magnitude at which the curve starts.
    pub const DEADZONE: &str = "curve.deadzone";
    /// Exponent offset near the centre.
    pub const CENTER_EXPO: &str = "curve.center_expo";
    /// Exponent offset near full input.
    pub const OUTER_EXPO: &str = "curve.outer_expo";
    /// Maximum demand acceleration while the input is active.
    pub const APPLY_ACCEL: &str = "dynamics.apply_accel";
    /// Maximum demand jerk while the input is active.
    pub const APPLY_JERK: &str = "dynamics.apply_jerk";
    /// How much prompter a release is than an apply.
    pub const RELEASE_FACTOR: &str = "dynamics.release_factor";
    /// Curved magnitude that changes a neutral input to active.
    pub const NEUTRAL_ENTER: &str = "neutral.active_enter";
    /// Continuous neutral interval before release, in milliseconds.
    pub const NEUTRAL_DWELL_MS: &str = "neutral.dwell_ms";
}

/// The vehicle the bench flies, reduced to one number.
#[derive(Debug, Clone, Copy)]
pub struct BenchVehicle {
    /// The velocity time constant, in seconds.
    pub time_constant_s: f64,
    /// The largest velocity the vehicle reaches at full demand, in m/s.
    pub full_scale_mps: f64,
}

impl BenchVehicle {
    /// The Alia 250: heavier, so slower to answer and faster at full scale.
    #[must_use]
    pub const fn alia250() -> Self {
        Self {
            time_constant_s: 0.45,
            full_scale_mps: 3.0,
        }
    }

    /// The x500: light and quick.
    #[must_use]
    pub const fn x500() -> Self {
        Self {
            time_constant_s: 0.18,
            full_scale_mps: 5.0,
        }
    }
}

/// State the vehicle adapter writes and the backend reads.
#[derive(Debug, Default)]
struct Settled {
    response: Option<AxisResponse>,
    digest: Option<Digest>,
    /// Receipts the run sealed, kept so a recovery can return them.
    sealed: Vec<flight_tune::RunTerminalReceipt>,
}

/// The active candidate, shared between the two halves of the contract.
#[derive(Debug, Clone, Default)]
pub struct BenchHandle(Rc<RefCell<Settled>>);

/// Reads one candidate's parameters as an axis response.
///
/// # Errors
///
/// Returns [`AdapterError`] when a parameter is absent or not finite.
fn response_from(candidate: &Candidate) -> Result<AxisResponse, AdapterError> {
    let read = |name: &str| -> Result<f64, AdapterError> {
        candidate
            .parameters()
            .get(name)
            .copied()
            .filter(|value| value.is_finite())
            .ok_or_else(|| AdapterError::new(format!("the candidate states no {name}")))
    };
    let apply_accel = read(parameter::APPLY_ACCEL)?;
    let apply_jerk = read(parameter::APPLY_JERK)?;
    let release_factor = read(parameter::RELEASE_FACTOR)?;
    let enter = read(parameter::NEUTRAL_ENTER)?;
    let center_expo = read(parameter::CENTER_EXPO)? as f32;
    Ok(AxisResponse {
        curve: AxisCurve {
            deadzone: read(parameter::DEADZONE)? as f32,
            center_expo,
            // The profile validator refuses an outer expo above the
            // center one; folding the search there keeps every sealed
            // winner a law a real profile will load. The outer blend
            // begins where the shipped law's does, so the trial's firm
            // input exercises it.
            outer_expo: (read(parameter::OUTER_EXPO)? as f32).min(center_expo),
            outer_start: 0.7,
        },
        neutral: NeutralBand {
            active_enter: enter as f32,
            // Leaving is harder than staying, or an input on the edge chatters.
            // The search sets the entry and the exit follows it, so no
            // candidate can propose a band with no hysteresis in it.
            active_exit: (enter * 0.65) as f32,
            dwell_ms: read(parameter::NEUTRAL_DWELL_MS)?.max(0.0) as u32,
        },
        dynamics: AxisDynamics {
            apply_accel: apply_accel as f32,
            apply_jerk: apply_jerk as f32,
            // Letting go is never slower than asking, whatever the search
            // proposes: a release that lagged the apply would take longer to
            // stop commanding than to start.
            release_accel: (apply_accel * release_factor.max(1.0)) as f32,
            release_jerk: (apply_jerk * release_factor.max(1.0)) as f32,
            reversal_accel: apply_accel as f32,
            reversal_jerk: apply_jerk as f32,
        },
    })
}

/// The simulator half: it flies the trial and reports the channels.
#[derive(Debug)]
pub struct BenchBackend {
    handle: BenchHandle,
    vehicle_model: BenchVehicle,
    simulator: ArtifactIdentity,
    airframe: ArtifactIdentity,
    run: Option<Run>,
}

#[derive(Debug)]
struct Run {
    shaper: AxisDemandShaper,
    response: AxisResponse,
    step: u32,
    velocity: f64,
    position: f64,
    previous_velocity: f64,
}

impl BenchBackend {
    /// Creates one backend for a named vehicle model.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when an identity cannot be built.
    pub fn new(
        handle: BenchHandle,
        vehicle_model: BenchVehicle,
        airframe_id: &str,
        airframe_digest: Digest,
    ) -> Result<Self, AdapterError> {
        let build = |name: &str, digest: Digest| {
            ArtifactIdentity::new(name, digest)
                .map_err(|error| AdapterError::new(error.to_string()))
        };
        Ok(Self {
            handle,
            vehicle_model,
            simulator: build("pilotage-control-feel-bench", Digest::from_bytes([1; 32]))?,
            airframe: build(airframe_id, airframe_digest)?,
            run: None,
        })
    }

    /// The phase and raw stick input at one trial time.
    ///
    /// The step is a FIRM input, not a full-scale one. At full scale the
    /// curve maps every legal deadzone and expo to the same output, so
    /// half the searched parameters cannot move a single sample — and
    /// tracking a deliberate full-scale hold reads as one long
    /// saturation stretch no legal candidate can avoid, which turns the
    /// saturation ceiling into a bar nothing can pass. At this level
    /// the curve shapes the demand, the neutral latch sees a real
    /// crossing, and saturation means what the ceiling says it means.
    fn input_at(time_s: f64) -> (f64, f32) {
        match time_s {
            t if t < STEP_AT_S => (0.0, 0.0),
            t if t < HOLD_AT_S => (1.0, 0.85),
            t if t < RELEASE_AT_S => (2.0, 0.85),
            t if t < SETTLE_AT_S => (3.0, 0.0),
            _ => (4.0, 0.0),
        }
    }
}

impl SimulatorBackend for BenchBackend {
    fn simulator_identity(&self) -> &ArtifactIdentity {
        &self.simulator
    }

    fn airframe_identity(&self) -> &ArtifactIdentity {
        &self.airframe
    }

    fn open_session_blocking(
        &mut self,
        challenge: &SessionChallenge,
    ) -> Result<SimulatorSessionReceipt, AdapterError> {
        Ok(SimulatorSessionReceipt {
            session_digest: challenge.session_digest(),
            simulator_digest: self.simulator.digest,
            airframe_digest: self.airframe.digest,
        })
    }

    fn prepare_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
        scenario: &ScenarioRef,
    ) -> Result<RunPreparationReceipt, AdapterError> {
        let _ = scenario;
        Ok(RunPreparationReceipt {
            session_digest: capability.session_digest(),
            run_intent_digest: context.digest().map_err(to_adapter)?,
        })
    }

    fn start_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
    ) -> Result<ScenarioStartReceipt, AdapterError> {
        let response = self
            .handle
            .0
            .borrow()
            .response
            .ok_or_else(|| AdapterError::new("no candidate has been settled"))?;
        // Receipts do not outlive the run that sealed them. Returning an
        // earlier run's receipts to a later recovery is returning a foreign
        // receipt, and the engine refuses one.
        self.handle.0.borrow_mut().sealed.clear();
        self.run = Some(Run {
            shaper: AxisDemandShaper::default(),
            response,
            step: 0,
            velocity: 0.0,
            position: 0.0,
            previous_velocity: 0.0,
        });
        Ok(ScenarioStartReceipt {
            session_digest: capability.session_digest(),
            applied_scenario_digest: context.scenario_digest(),
            seed: context.seed(),
            run_intent_digest: context.digest().map_err(to_adapter)?,
        })
    }

    fn sample_blocking(&mut self, _timeout: Duration) -> Result<SampleEvent, AdapterError> {
        let model = self.vehicle_model;
        let Some(run) = self.run.as_mut() else {
            return Err(AdapterError::new("no scenario is running"));
        };
        let time_s = f64::from(run.step) * DT_S;
        if time_s >= END_S {
            return Ok(SampleEvent::Complete);
        }
        let (phase, stick) = Self::input_at(time_s);
        let shaped = run.shaper.step(stick, 1.0, DT_S as f32, run.response).value;
        let demanded_mps = f64::from(shaped) * model.full_scale_mps;
        run.velocity += (demanded_mps - run.velocity) * DT_S / model.time_constant_s;
        run.position += run.velocity * DT_S;
        let acceleration = (run.velocity - run.previous_velocity) / DT_S;
        run.previous_velocity = run.velocity;
        let sample = TelemetrySample {
            sequence: u64::from(run.step),
            elapsed_ms: u64::from(run.step) * 20,
            values: BTreeMap::from([
                (channel::COMMAND.to_owned(), f64::from(shaped)),
                (
                    channel::RESPONSE.to_owned(),
                    run.velocity / model.full_scale_mps,
                ),
                (channel::POSITION_M.to_owned(), run.position),
                (channel::VELOCITY_MPS.to_owned(), run.velocity),
                (channel::ACCELERATION_MPS2.to_owned(), acceleration),
                (channel::EFFORT.to_owned(), f64::from(shaped.abs())),
                (
                    channel::SATURATED.to_owned(),
                    f64::from(u8::from(shaped.abs() >= 0.999)),
                ),
                (channel::PHASE.to_owned(), phase),
            ]),
        };
        run.step = run.step.wrapping_add(1);
        Ok(SampleEvent::Sample(sample))
    }

    fn stop_blocking(&mut self) -> Result<(), AdapterError> {
        self.run = None;
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), AdapterError> {
        self.run = None;
        Ok(())
    }
}

/// Reads an engine error as an adapter one.
pub(super) fn to_adapter(error: flight_tune::TuneError) -> AdapterError {
    AdapterError::new(error.to_string())
}

mod adapter;
mod qualifying;

pub use adapter::BenchVehicleAdapter;
pub use qualifying::{
    BenchGates, BenchVehicleFactory, bench_scenario, bench_stage, warm_start_parameters,
};

#[cfg(test)]
mod tests;

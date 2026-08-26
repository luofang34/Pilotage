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
//! SIM / NOT FOR FLIGHT.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use flight_tune::{
    AdapterError, ArtifactIdentity, Candidate, CandidateReceipt, CandidateTransitionReceipt,
    CandidateTransitionRequest, Digest, EvaluatorError, GateEvaluator, GateOutcome,
    RunExecutionContext, RunPreparationReceipt, SampleEvent, ScenarioRef, ScenarioStartReceipt,
    SessionChallenge, SimulatorBackend, SimulatorCapability, SimulatorSessionReceipt,
    SimulatorVehicleAdapter, SimulatorVehicleFactory, TelemetrySample, TransitionBindingReceipt,
    VehicleBinding, VehicleBindingReceipt,
};
use pilotage_control_feel::{
    AxisCurve, AxisDemandShaper, AxisDynamics, AxisResponse, FeelMode, FlightFeelProfile,
    NeutralBand,
};

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
    Ok(AxisResponse {
        curve: AxisCurve {
            deadzone: read(parameter::DEADZONE)? as f32,
            center_expo: read(parameter::CENTER_EXPO)? as f32,
            outer_expo: read(parameter::OUTER_EXPO)? as f32,
            outer_start: 1.0,
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

/// The vehicle half of the contract: it settles a candidate's command law.
#[derive(Debug)]
pub struct BenchVehicleAdapter {
    handle: BenchHandle,
}

impl BenchVehicleAdapter {
    /// Creates one adapter over shared state.
    #[must_use]
    pub fn new(handle: BenchHandle) -> Self {
        Self { handle }
    }
}

impl SimulatorVehicleAdapter for BenchVehicleAdapter {
    fn authorize_candidate_transition(
        &self,
        request: &CandidateTransitionRequest,
    ) -> Result<CandidateTransitionReceipt, AdapterError> {
        CandidateTransitionReceipt::authorized(request)
            .map_err(|error| AdapterError::new(error.to_string()))
    }

    fn ensure_settled_candidate_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        let mut settled = self.handle.0.borrow_mut();
        // Repeating the request must not rewrite the law, so a restart can be
        // reconciled without disturbing a vehicle already on the candidate.
        if settled.digest != Some(candidate_digest) {
            settled.response = Some(response_from(candidate)?);
            settled.digest = Some(candidate_digest);
        }
        Ok(CandidateReceipt {
            session_digest: _capability.session_digest(),
            requested_digest: candidate_digest,
            applied_digest: candidate_digest,
            // The bench applies the law in process, so what it reads back is
            // what it applied. A vehicle over a link reads its controller.
            readback_digest: candidate_digest,
            // Idle reconciliation settles a candidate without a run behind it.
            run_intent_digest: None,
        })
    }

    fn ensure_candidate_for_run_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        let mut receipt =
            self.ensure_settled_candidate_blocking(capability, candidate, candidate_digest)?;
        // The receipt names the run it was settled for, so a law applied for
        // one run cannot be read as evidence for another.
        receipt.run_intent_digest = Some(context.digest().map_err(to_adapter)?);
        Ok(receipt)
    }
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
    fn input_at(time_s: f64) -> (f64, f32) {
        match time_s {
            t if t < STEP_AT_S => (0.0, 0.0),
            t if t < HOLD_AT_S => (1.0, 1.0),
            t if t < RELEASE_AT_S => (2.0, 1.0),
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
        _capability: &SimulatorCapability,
        _context: &RunExecutionContext,
        scenario: &ScenarioRef,
    ) -> Result<RunPreparationReceipt, AdapterError> {
        let _ = scenario;
        Ok(RunPreparationReceipt {
            session_digest: _capability.session_digest(),
            run_intent_digest: _context.digest().map_err(to_adapter)?,
        })
    }

    fn start_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _context: &RunExecutionContext,
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
            session_digest: _capability.session_digest(),
            applied_scenario_digest: _context.scenario_digest(),
            seed: _context.seed(),
            run_intent_digest: _context.digest().map_err(to_adapter)?,
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

impl flight_tune::RunTerminalAdapter for BenchVehicleAdapter {
    fn terminal_capabilities(&self) -> flight_tune::RunTerminalCapabilities {
        // The bench holds no external control path, no trace collector and no
        // supervised child: the law, the vehicle and the trace are all in this
        // process, and a run ends when the loop stops stepping. Advertising a
        // capability it does not have would have the engine wait for a stop
        // acknowledgement nothing will send.
        flight_tune::RunTerminalCapabilities::new(false, false, false)
    }

    fn bind_terminal_plan_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &flight_tune::RunBindingReceipt,
        _plan: &flight_tune::RunTerminalPlan,
    ) -> Result<(), AdapterError> {
        // Nothing to bind: with no external component to stop, the plan is
        // empty and accepting it is the whole of the work. Repeating the call
        // must be safe, and doing nothing twice is.
        Ok(())
    }

    fn causal_evidence_digest_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &flight_tune::RunBindingReceipt,
    ) -> Result<Digest, AdapterError> {
        // The causal evidence for an in-process run is the run itself: there
        // is no external trace to correlate against, so the identity stated
        // here is the one fixed value that says which trace this was.
        Ok(Digest::from_bytes([0x0c; 32]))
    }

    fn seal_terminal_receipt_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &flight_tune::RunBindingReceipt,
        receipt: &flight_tune::RunTerminalReceipt,
    ) -> Result<(), AdapterError> {
        // A seal the adapter acknowledges and does not keep is a seal the
        // engine cannot read back, and the engine is right to refuse one.
        // Sealing the same receipt twice keeps one copy, so a repeated call
        // after an uncertain acknowledgement is safe.
        let mut settled = self.handle.0.borrow_mut();
        if !settled.sealed.contains(receipt) {
            settled.sealed.push(receipt.clone());
        }
        Ok(())
    }

    fn recover_terminal_receipts_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &flight_tune::RunBindingReceipt,
    ) -> Result<Vec<flight_tune::RunTerminalReceipt>, AdapterError> {
        Ok(self.handle.0.borrow().sealed.clone())
    }
}

/// Reads an engine error as an adapter one.
fn to_adapter(error: flight_tune::TuneError) -> AdapterError {
    AdapterError::new(error.to_string())
}

/// Binds the bench vehicle to a validated simulator session.
#[derive(Debug)]
pub struct BenchVehicleFactory {
    handle: BenchHandle,
    identity: ArtifactIdentity,
    transition_validator: ArtifactIdentity,
    adjacency_policy_digest: Digest,
}

impl BenchVehicleFactory {
    /// Creates one factory for a named vehicle.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError`] when an identity cannot be built.
    pub fn new(handle: BenchHandle, vehicle_id: &str) -> Result<Self, AdapterError> {
        let text = |value: &str| {
            ArtifactIdentity::from_text(value, value)
                .map_err(|error| AdapterError::new(error.to_string()))
        };
        Ok(Self {
            handle,
            identity: text(vehicle_id)?,
            transition_validator: text("bench-transition-validator")?,
            adjacency_policy_digest: Digest::from_bytes([8; 32]),
        })
    }
}

impl SimulatorVehicleFactory for BenchVehicleFactory {
    type Adapter = BenchVehicleAdapter;

    fn vehicle_identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn transition_validator_identity(&self) -> &ArtifactIdentity {
        &self.transition_validator
    }

    fn adjacency_policy_digest(&self) -> Digest {
        self.adjacency_policy_digest
    }

    fn bind_blocking(
        self,
        capability: &SimulatorCapability,
    ) -> Result<VehicleBinding<Self::Adapter>, AdapterError> {
        let transition = TransitionBindingReceipt::new(
            capability.session_digest(),
            self.transition_validator,
            self.adjacency_policy_digest,
        )?;
        capability.bind_vehicle_with_transition(
            BenchVehicleAdapter::new(self.handle),
            VehicleBindingReceipt {
                session_digest: capability.session_digest(),
                vehicle_digest: self.identity.digest,
            },
            transition,
        )
    }
}

/// The hard gates a bench run must not trip.
///
/// A gate is a refusal, not a score: a run that trips one is not a worse
/// candidate, it is a candidate whose result means nothing. Speed and
/// acceleration ceilings here stand for the envelope a real vehicle would be
/// held inside.
#[derive(Debug)]
pub struct BenchGates {
    identity: ArtifactIdentity,
    maximum_speed_mps: f64,
    maximum_acceleration_mps2: f64,
}

impl BenchGates {
    /// Creates gates at one vehicle's ceilings.
    ///
    /// # Errors
    ///
    /// Returns [`EvaluatorError`] when the identity cannot be built.
    pub fn new(
        maximum_speed_mps: f64,
        maximum_acceleration_mps2: f64,
    ) -> Result<Self, EvaluatorError> {
        let identity = ArtifactIdentity::from_text("bench-envelope-gates", "bench-envelope-gates")
            .map_err(|error| EvaluatorError::new(error.to_string()))?;
        Ok(Self {
            identity,
            maximum_speed_mps,
            maximum_acceleration_mps2,
        })
    }
}

impl GateEvaluator for BenchGates {
    fn identity(&self) -> &ArtifactIdentity {
        &self.identity
    }

    fn begin(&mut self, _scenario: &ScenarioRef) -> Result<(), EvaluatorError> {
        Ok(())
    }

    fn evaluate(&mut self, sample: &TelemetrySample) -> Result<Vec<GateOutcome>, EvaluatorError> {
        let read = |name: &str| sample.values.get(name).copied().unwrap_or(0.0);
        let speed = read(channel::VELOCITY_MPS).abs();
        let acceleration = read(channel::ACCELERATION_MPS2).abs();
        let mut outcomes = Vec::new();
        outcomes.push(if speed <= self.maximum_speed_mps {
            GateOutcome::pass("envelope.speed")
        } else {
            GateOutcome::fail("envelope.speed", format!("{speed:.3} m/s"))
        });
        if !outcomes[0].passed {
            return Ok(outcomes);
        }
        outcomes.push(if acceleration <= self.maximum_acceleration_mps2 {
            GateOutcome::pass("envelope.acceleration")
        } else {
            GateOutcome::fail("envelope.acceleration", format!("{acceleration:.3} m/s2"))
        });
        Ok(outcomes)
    }

    fn finish(&mut self) -> Result<(), EvaluatorError> {
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), EvaluatorError> {
        Ok(())
    }
}

/// The starting command law a search departs from, as candidate parameters.
///
/// This is the shaped mode the vehicle would otherwise ship with, expressed in
/// the parameters the search may move. Starting a search from a law that is
/// already safe is what keeps the first trials from being wasted on finding
/// one.
#[must_use]
pub fn warm_start_parameters(mode: FeelMode) -> BTreeMap<String, f64> {
    let profile = FlightFeelProfile::shaped(mode);
    let axis = profile.horizontal;
    BTreeMap::from([
        (
            parameter::DEADZONE.to_owned(),
            f64::from(axis.curve.deadzone),
        ),
        (
            parameter::CENTER_EXPO.to_owned(),
            f64::from(axis.curve.center_expo),
        ),
        (
            parameter::OUTER_EXPO.to_owned(),
            f64::from(axis.curve.outer_expo),
        ),
        (
            parameter::APPLY_ACCEL.to_owned(),
            f64::from(axis.dynamics.apply_accel),
        ),
        (
            parameter::APPLY_JERK.to_owned(),
            f64::from(axis.dynamics.apply_jerk),
        ),
        (
            parameter::RELEASE_FACTOR.to_owned(),
            f64::from(axis.dynamics.release_accel / axis.dynamics.apply_accel),
        ),
        (
            parameter::NEUTRAL_ENTER.to_owned(),
            f64::from(axis.neutral.active_enter),
        ),
        (
            parameter::NEUTRAL_DWELL_MS.to_owned(),
            f64::from(axis.neutral.dwell_ms),
        ),
    ])
}

/// The stage one vehicle is tuned over.
///
/// The allowlist is the command law's own numbers, bounded around the shaped
/// starting point rather than around zero: a search that may propose any value
/// spends its first trials rediscovering that a control has to be stable,
/// which is what a warm start exists to avoid.
#[must_use]
pub fn bench_stage(
    id: &str,
    promotion: flight_tune::PromotionPolicy,
    qualification: flight_tune::QualificationPolicy,
) -> flight_tune::SearchStage {
    use flight_tune::ParameterBounds;
    let bounds = |minimum: f64, maximum: f64| ParameterBounds { minimum, maximum };
    flight_tune::SearchStage {
        id: id.to_owned(),
        allowlist: BTreeMap::from([
            (parameter::DEADZONE.to_owned(), bounds(0.02, 0.12)),
            (parameter::CENTER_EXPO.to_owned(), bounds(0.0, 0.7)),
            (parameter::OUTER_EXPO.to_owned(), bounds(0.0, 0.7)),
            (parameter::APPLY_ACCEL.to_owned(), bounds(1.0, 10.0)),
            (parameter::APPLY_JERK.to_owned(), bounds(5.0, 80.0)),
            (parameter::RELEASE_FACTOR.to_owned(), bounds(1.0, 2.0)),
            (parameter::NEUTRAL_ENTER.to_owned(), bounds(0.01, 0.06)),
            (parameter::NEUTRAL_DWELL_MS.to_owned(), bounds(20.0, 200.0)),
        ]),
        fixed_parameters: BTreeMap::new(),
        required_hard_gates: vec![
            "envelope.speed".to_owned(),
            "envelope.acceleration".to_owned(),
        ],
        // Training, promotion and final qualification fly separate scenarios.
        // The bar that decides what ships is measured on runs the search never
        // saw, so a candidate cannot be fitted to the scenario that judges it.
        training_scenarios: vec![bench_scenario("training-step", 21)],
        promotion_scenarios: vec![bench_scenario("promotion-step", 22)],
        final_qualification_scenarios: vec![bench_scenario("final-step", 23)],
        repetitions: 2,
        promotion,
        qualification,
    }
}

/// One trial of the bench, as a scenario reference.
#[must_use]
pub fn bench_scenario(id: &str, digest_byte: u8) -> ScenarioRef {
    ScenarioRef {
        id: id.to_owned(),
        digest: Digest::from_bytes([digest_byte; 32]),
        // The trial is nine seconds at fifty hertz, and the ceiling is the
        // sample count that covers it with room for the completion event.
        max_samples: 700,
        sample_timeout_ms: 200,
    }
}

#[cfg(test)]
mod tests;

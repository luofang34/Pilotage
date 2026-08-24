//! Versioned samples for one trial trace.

mod control;
mod missing;
mod stage;
mod state;
mod stream;
mod time;

use serde::{Deserialize, Serialize};

use crate::{
    ClockDomain, CodecError, Digest, MAX_SAMPLE_BYTES, RunIdentity, TRIAL_SAMPLE_SCHEMA_VERSION,
    ValidationError, canonical,
    validation::{digest, schema},
};

pub use control::{AdapterDisposition, ControlAxes, ControlValue, RawInput, ReferenceFrame};
pub use missing::{MissingReason, MissingSignal, Observed};
pub use stage::{
    CausalStage, ControlEventId, ControlStage, SourceStamp, StageProducerRole, StageStamp,
};
pub use state::{
    ActuatorState, ConditionState, HealthState, KinematicState, LifecycleObservation,
    LifecycleState, NamedValue, Quaternion, SimulatorTruthEvidence, Vector3,
};
pub use stream::TrialStreamValidator;
pub use time::{ClockReading, SampleTime};

/// One versioned sample in a trial trace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialSample {
    /// The trial sample schema version.
    pub schema_version: u16,
    /// The canonical digest of the run identity.
    pub run_digest: Digest,
    /// The monotonic sequence number in this run.
    pub sequence: u64,
    /// The number of lost samples before this sample.
    pub dropped_before: u32,
    /// The zero-based scenario phase index.
    pub phase_index: u16,
    /// The sample times in all available clocks.
    pub time: SampleTime,
    /// The raw device input.
    pub raw_input: CausalStage<RawInput>,
    /// The normalized input channels.
    pub normalized_control: CausalStage<ControlAxes>,
    /// The typed control intent.
    pub typed_intent: CausalStage<ControlValue>,
    /// The demand that the adapter receives.
    pub adapter_demand: CausalStage<ControlValue>,
    /// The setpoint that the adapter transmits.
    pub transmitted_setpoint: CausalStage<ControlValue>,
    /// The flight controller kinematic estimate.
    pub flight_controller_estimate: CausalStage<KinematicState>,
    /// The simulator kinematic truth evidence.
    pub simulator_truth: CausalStage<SimulatorTruthEvidence>,
    /// The actuator effort and saturation state.
    pub actuator: Observed<ActuatorState>,
    /// The adapter decision for the control demand.
    pub adapter_disposition: Observed<AdapterDisposition>,
    /// The simulator and vehicle lifecycle state.
    pub lifecycle: Observed<LifecycleObservation>,
    /// The measured environmental condition state.
    pub condition_state: Observed<ConditionState>,
    /// The control link validity state.
    pub link_state: Observed<HealthState>,
    /// The flight controller estimator validity state.
    pub estimator_state: Observed<HealthState>,
}

impl TrialSample {
    /// Decodes a sample and validates its run identity link.
    pub fn from_json_for_run(bytes: &[u8], run: &RunIdentity) -> Result<Self, CodecError> {
        let value: Self = canonical::decode("trial sample", bytes, MAX_SAMPLE_BYTES)?;
        value.validate_for_run(run)?;
        Ok(value)
    }

    /// Validates local content without a run identity link.
    pub fn validate_local(&self) -> Result<(), ValidationError> {
        schema(
            "trial sample",
            self.schema_version,
            TRIAL_SAMPLE_SCHEMA_VERSION,
        )?;
        digest("sample.run_digest", self.run_digest)?;
        self.time.validate()?;
        self.validate_causal_stages_local()?;
        self.validate_state_signals()?;
        self.validate_status_signals()
    }

    /// Validates the sample and its canonical run identity link.
    pub fn validate_for_run(&self, run: &RunIdentity) -> Result<(), CodecError> {
        run.validate()?;
        self.validate_local()?;
        if self.run_digest != run.canonical_digest()? {
            return Err(ValidationError::IdentityMismatch {
                field: "sample.run_digest".to_owned(),
            }
            .into());
        }
        self.time.validate_for_run(run)?;
        self.validate_causal_stage_clocks(run)?;
        self.validate_control_lineage_shape()?;
        Ok(())
    }

    /// Validates this sample after one adjacent recorded sample in the same run.
    ///
    /// This operation does not retain earlier present values across a missing
    /// observation. Use [`TrialStreamValidator`] to validate a complete trace.
    pub fn validate_adjacent_only(
        &self,
        previous: &Self,
        run: &RunIdentity,
    ) -> Result<(), CodecError> {
        self.validate_after(previous, run)
    }

    pub(crate) fn validate_after(
        &self,
        previous: &Self,
        run: &RunIdentity,
    ) -> Result<(), CodecError> {
        previous.validate_local()?;
        self.validate_local()?;
        if previous.run_digest != self.run_digest {
            return Err(ValidationError::MixedRun.into());
        }
        previous.validate_for_run(run)?;
        self.validate_for_run(run)?;
        validate_sequence(previous, self)?;
        if self.phase_index < previous.phase_index {
            return Err(ValidationError::PhaseOrder {
                previous: previous.phase_index,
                current: self.phase_index,
            }
            .into());
        }
        validate_clock_order(previous, self)?;
        self.validate_causal_stage_order(previous)?;
        Ok(())
    }

    /// Encodes canonical compact JSON after run-bound validation.
    pub fn to_canonical_json_for_run(&self, run: &RunIdentity) -> Result<Vec<u8>, CodecError> {
        self.validate_for_run(run)?;
        canonical::encode("trial sample", self, MAX_SAMPLE_BYTES)
    }

    /// Calculates the digest of run-bound canonical compact JSON.
    pub fn canonical_digest_for_run(&self, run: &RunIdentity) -> Result<Digest, CodecError> {
        self.to_canonical_json_for_run(run)
            .map(|bytes| canonical::digest(&bytes))
    }

    fn validate_causal_stages_local(&self) -> Result<(), ValidationError> {
        let recorder_ns = self.time.recorder_monotonic_ns;
        self.raw_input.validate_local_with(
            "sample.raw_input",
            StageProducerRole::InputCapture,
            recorder_ns,
            RawInput::validate,
        )?;
        self.normalized_control.validate_local_with(
            "sample.normalized_control",
            StageProducerRole::ControlClient,
            recorder_ns,
            ControlAxes::validate,
        )?;
        self.typed_intent.validate_local_with(
            "sample.typed_intent",
            StageProducerRole::ControlClient,
            recorder_ns,
            ControlValue::validate,
        )?;
        self.adapter_demand.validate_local_with(
            "sample.adapter_demand",
            StageProducerRole::ControlClient,
            recorder_ns,
            ControlValue::validate,
        )?;
        self.transmitted_setpoint.validate_local_with(
            "sample.transmitted_setpoint",
            StageProducerRole::VehicleAdapter,
            recorder_ns,
            ControlValue::validate,
        )?;
        self.flight_controller_estimate.validate_local_with(
            "sample.flight_controller_estimate",
            StageProducerRole::FlightController,
            recorder_ns,
            KinematicState::validate,
        )?;
        self.simulator_truth.validate_local_with(
            "sample.simulator_truth",
            StageProducerRole::SimulatorBackend,
            recorder_ns,
            SimulatorTruthEvidence::validate,
        )
    }

    fn validate_causal_stage_clocks(&self, run: &RunIdentity) -> Result<(), ValidationError> {
        self.raw_input.validate_clock("sample.raw_input", run)?;
        self.normalized_control
            .validate_clock("sample.normalized_control", run)?;
        self.typed_intent
            .validate_clock("sample.typed_intent", run)?;
        self.adapter_demand
            .validate_clock("sample.adapter_demand", run)?;
        self.transmitted_setpoint
            .validate_clock("sample.transmitted_setpoint", run)?;
        self.flight_controller_estimate
            .validate_clock("sample.flight_controller_estimate", run)?;
        self.simulator_truth
            .validate_clock("sample.simulator_truth", run)
    }

    fn validate_control_lineage_shape(&self) -> Result<(), ValidationError> {
        self.raw_input
            .validate_predecessor_stage("sample.raw_input", None)?;
        self.normalized_control.validate_predecessor_stage(
            "sample.normalized_control",
            Some(ControlStage::RawInput),
        )?;
        self.typed_intent.validate_predecessor_stage(
            "sample.typed_intent",
            Some(ControlStage::NormalizedControl),
        )?;
        self.adapter_demand
            .validate_predecessor_stage("sample.adapter_demand", Some(ControlStage::TypedIntent))?;
        self.transmitted_setpoint.validate_predecessor_stage(
            "sample.transmitted_setpoint",
            Some(ControlStage::AdapterDemand),
        )?;
        self.flight_controller_estimate
            .validate_predecessor_stage("sample.flight_controller_estimate", None)?;
        self.simulator_truth
            .validate_predecessor_stage("sample.simulator_truth", None)
    }

    fn validate_causal_stage_order(&self, previous: &Self) -> Result<(), ValidationError> {
        let discontinuity = |stage: &StageStamp| self.time.has_discontinuity(stage.source.clock);
        self.raw_input.validate_after(
            &previous.raw_input,
            "sample.raw_input",
            discontinuity(&self.raw_input.stamp),
        )?;
        self.normalized_control.validate_after(
            &previous.normalized_control,
            "sample.normalized_control",
            discontinuity(&self.normalized_control.stamp),
        )?;
        self.typed_intent.validate_after(
            &previous.typed_intent,
            "sample.typed_intent",
            discontinuity(&self.typed_intent.stamp),
        )?;
        self.adapter_demand.validate_after(
            &previous.adapter_demand,
            "sample.adapter_demand",
            discontinuity(&self.adapter_demand.stamp),
        )?;
        self.transmitted_setpoint.validate_after(
            &previous.transmitted_setpoint,
            "sample.transmitted_setpoint",
            discontinuity(&self.transmitted_setpoint.stamp),
        )?;
        self.flight_controller_estimate.validate_after(
            &previous.flight_controller_estimate,
            "sample.flight_controller_estimate",
            discontinuity(&self.flight_controller_estimate.stamp),
        )?;
        self.simulator_truth.validate_after(
            &previous.simulator_truth,
            "sample.simulator_truth",
            discontinuity(&self.simulator_truth.stamp),
        )
    }

    fn validate_state_signals(&self) -> Result<(), ValidationError> {
        self.actuator
            .validate_with("sample.actuator", ActuatorState::validate)?;
        self.condition_state
            .validate_with("sample.condition_state", ConditionState::validate)
    }

    fn validate_status_signals(&self) -> Result<(), ValidationError> {
        self.adapter_disposition
            .validate_with("sample.adapter_disposition", AdapterDisposition::validate)?;
        self.lifecycle
            .validate_with("sample.lifecycle", LifecycleObservation::validate)?;
        self.link_state
            .validate_with("sample.link_state", HealthState::validate)?;
        self.estimator_state
            .validate_with("sample.estimator_state", HealthState::validate)
    }
}

fn validate_sequence(previous: &TrialSample, current: &TrialSample) -> Result<(), ValidationError> {
    if current.sequence <= previous.sequence {
        return Err(ValidationError::SequenceOrder {
            previous: previous.sequence,
            current: current.sequence,
        });
    }
    let increment = u64::from(current.dropped_before).wrapping_add(1);
    let Some(expected) = previous.sequence.checked_add(increment) else {
        return Err(ValidationError::SequenceOrder {
            previous: previous.sequence,
            current: current.sequence,
        });
    };
    if current.sequence != expected {
        return Err(ValidationError::SequenceGap {
            expected,
            actual: current.sequence,
        });
    }
    Ok(())
}

fn validate_clock_order(
    previous: &TrialSample,
    current: &TrialSample,
) -> Result<(), ValidationError> {
    if current.time.recorder_monotonic_ns < previous.time.recorder_monotonic_ns {
        return Err(ValidationError::ClockRegression {
            clock: format!("{:?}", ClockDomain::Recorder),
            previous_ns: previous.time.recorder_monotonic_ns,
            current_ns: current.time.recorder_monotonic_ns,
        });
    }
    const DOMAINS: [ClockDomain; 5] = [
        ClockDomain::Device,
        ClockDomain::Client,
        ClockDomain::Adapter,
        ClockDomain::FlightController,
        ClockDomain::Simulator,
    ];
    for domain in DOMAINS {
        let Some(prior) = previous.time.source_reading(domain) else {
            continue;
        };
        let Some(next) = current.time.source_reading(domain) else {
            continue;
        };
        let has_discontinuity = current.time.has_discontinuity(domain);
        if prior.epoch == next.epoch && next.time_ns < prior.time_ns {
            return Err(ValidationError::ClockRegression {
                clock: format!("{domain:?}"),
                previous_ns: prior.time_ns,
                current_ns: next.time_ns,
            });
        }
        if prior.epoch == next.epoch && has_discontinuity {
            return invalid_clock_observation(
                "sample.time.clock_discontinuities",
                "a clock discontinuity must change the source epoch",
            );
        }
        if prior.epoch != next.epoch && !has_discontinuity {
            return invalid_clock_observation(
                "sample.time.clock_discontinuities",
                "a source epoch change needs a clock discontinuity",
            );
        }
    }
    Ok(())
}

fn invalid_clock_observation<T>(field: &str, reason: &'static str) -> Result<T, ValidationError> {
    Err(ValidationError::InvalidClockObservation {
        field: field.to_owned(),
        reason,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;

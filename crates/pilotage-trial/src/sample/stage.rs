//! Attributable stamps for each causal stage.

use serde::{Deserialize, Serialize};

use crate::{ClockDomain, MissingReason, Observed, RunIdentity, ValidationError};

/// The producer role for one causal stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageProducerRole {
    /// The input capture producer.
    InputCapture,
    /// The control client producer.
    ControlClient,
    /// The vehicle adapter producer.
    VehicleAdapter,
    /// The flight controller producer.
    FlightController,
    /// The simulator backend producer.
    SimulatorBackend,
}

/// One stage in the direct control-event chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlStage {
    /// The raw input stage.
    RawInput,
    /// The normalized control stage.
    NormalizedControl,
    /// The typed intent stage.
    TypedIntent,
    /// The adapter demand stage.
    AdapterDemand,
    /// The transmitted setpoint stage.
    TransmittedSetpoint,
}

/// The identity of one event in the direct control-event chain.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEventId {
    /// The control stage that produced the event.
    pub stage: ControlStage,
    /// The source clock domain.
    pub clock: ClockDomain,
    /// The source clock epoch.
    pub epoch: u64,
    /// The source event sequence number.
    pub sequence: u64,
}

/// The source identity and time for one causal stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceStamp {
    /// The producer role.
    pub producer: StageProducerRole,
    /// The source clock domain.
    pub clock: ClockDomain,
    /// The source clock epoch.
    pub epoch: u64,
    /// The source event or explicit missing-record sequence number in this epoch.
    pub sequence: u64,
    /// The event time or an explicit missing-time record.
    pub time_ns: Observed<u64>,
}

/// Source, recorder receive, and recorder apply times for one stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageStamp {
    /// The source stamp.
    pub source: SourceStamp,
    /// The direct predecessor for a present derived control event.
    pub predecessor: Option<ControlEventId>,
    /// The recorder time when it received this observation record.
    pub recorder_receive_ns: u64,
    /// The recorder time when it first made this observation current in the trace.
    pub recorder_apply_ns: u64,
}

/// One causal stage with its stamp and explicit observation state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CausalStage<T> {
    /// The attributable stage stamp.
    pub stamp: StageStamp,
    /// The stage value or an explicit missing-signal record.
    pub observation: Observed<T>,
}

impl<T> CausalStage<T> {
    /// Creates a present causal stage.
    #[must_use]
    pub const fn present(stamp: StageStamp, value: T) -> Self {
        Self {
            stamp,
            observation: Observed::present(value),
        }
    }

    /// Creates a missing causal stage.
    #[must_use]
    pub const fn missing(stamp: StageStamp, reason: MissingReason, detail: Option<String>) -> Self {
        Self {
            stamp,
            observation: Observed::missing(reason, detail),
        }
    }

    /// Gets the present stage value.
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        self.observation.value()
    }

    pub(crate) fn validate_local_with<F>(
        &self,
        field: &str,
        expected_producer: StageProducerRole,
        sample_recorder_ns: u64,
        validate: F,
    ) -> Result<(), ValidationError>
    where
        F: FnOnce(&T, &str) -> Result<(), ValidationError>,
    {
        if self.stamp.source.producer != expected_producer {
            return invalid_stamp(field, "the stage has an unexpected producer role");
        }
        self.stamp.validate(field, sample_recorder_ns)?;
        self.stamp
            .source
            .time_ns
            .validate_with(&format!("{field}.stamp.source.time_ns"), present_time)?;
        self.observation.validate_with(field, validate)
    }

    pub(crate) fn validate_clock(
        &self,
        field: &str,
        run: &RunIdentity,
    ) -> Result<(), ValidationError> {
        run.validate_stage_clock(
            field,
            self.stamp.source.clock,
            self.stamp.source.epoch,
            self.stamp.source.time_ns.value().copied(),
            self.stamp.recorder_receive_ns,
        )
    }

    pub(crate) fn validate_after(
        &self,
        previous: &Self,
        field: &str,
        has_clock_discontinuity: bool,
    ) -> Result<(), ValidationError>
    where
        T: PartialEq,
    {
        let current = &self.stamp.source;
        let prior = &previous.stamp.source;
        if current.clock != prior.clock {
            return invalid_stamp(field, "the source clock changes inside one run");
        }
        if current.epoch != prior.epoch {
            return if has_clock_discontinuity {
                Ok(())
            } else {
                invalid_stamp(
                    field,
                    "the source epoch changes without a clock discontinuity",
                )
            };
        }
        if has_clock_discontinuity {
            return invalid_stamp(field, "a clock discontinuity must change the source epoch");
        }
        if current.sequence < prior.sequence {
            return invalid_stamp(field, "the source sequence moves back in one epoch");
        }
        if current.sequence == prior.sequence
            && (self.stamp != previous.stamp || self.observation != previous.observation)
        {
            return invalid_stamp(field, "one source sequence has different content");
        }
        if matches!(
            (prior.time_ns.value(), current.time_ns.value()),
            (Some(prior_ns), Some(current_ns)) if current_ns < prior_ns
        ) {
            return invalid_stamp(field, "the source time moves back in one epoch");
        }
        Ok(())
    }

    pub(crate) fn validate_predecessor_stage(
        &self,
        field: &str,
        present_predecessor: Option<ControlStage>,
    ) -> Result<(), ValidationError> {
        let expected = self.observation.value().and(present_predecessor);
        if self.stamp.predecessor.map(|event| event.stage) == expected {
            return Ok(());
        }
        invalid_stamp(field, "the direct predecessor stage does not match")
    }
}

impl StageStamp {
    pub(crate) fn control_event_id(&self, stage: ControlStage) -> ControlEventId {
        ControlEventId {
            stage,
            clock: self.source.clock,
            epoch: self.source.epoch,
            sequence: self.source.sequence,
        }
    }

    fn validate(&self, field: &str, sample_recorder_ns: u64) -> Result<(), ValidationError> {
        if self.recorder_receive_ns > self.recorder_apply_ns {
            return invalid_stamp(field, "the recorder apply time is before the receive time");
        }
        if self.recorder_apply_ns > sample_recorder_ns {
            return invalid_stamp(field, "the sample time is before the recorder apply time");
        }
        if self.source.clock == ClockDomain::Recorder {
            if self.source.epoch != 0 {
                return invalid_stamp(field, "the recorder clock epoch must be zero");
            }
            if self
                .source
                .time_ns
                .value()
                .is_some_and(|time_ns| *time_ns > self.recorder_receive_ns)
            {
                return invalid_stamp(field, "the recorder receive time is before the source time");
            }
        }
        Ok(())
    }
}

fn present_time(_value: &u64, _field: &str) -> Result<(), ValidationError> {
    Ok(())
}

fn invalid_stamp(field: &str, reason: &'static str) -> Result<(), ValidationError> {
    Err(ValidationError::InvalidStageStamp {
        field: field.to_owned(),
        reason,
    })
}

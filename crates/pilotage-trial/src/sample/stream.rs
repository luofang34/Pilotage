//! Stateful validation for a complete trial sample stream.

use std::collections::VecDeque;

use crate::{ClockDomain, CodecError, MAX_CONTROL_EVENT_HISTORY, RunIdentity, ValidationError};

use super::{
    CausalStage, ClockReading, ControlEventId, ControlStage, SourceStamp, StageStamp, TrialSample,
};

/// Stateful validation for all recorded samples in one run.
///
/// This validator rejects a definite causal order violation. Mapping overlap
/// remains bounded evidence and does not prove an exact stage latency.
#[derive(Clone, Debug)]
pub struct TrialStreamValidator<'run> {
    run: &'run RunIdentity,
    previous: Option<TrialSample>,
    sample_clocks: SampleClockHistory,
    stage_times: StageTimeHistory,
    control_lineage: ControlLineageHistory,
    validated_samples: u64,
}

impl<'run> TrialStreamValidator<'run> {
    /// Creates an empty validator for one validated run identity.
    pub fn new(run: &'run RunIdentity) -> Result<Self, CodecError> {
        run.validate()?;
        Ok(Self {
            run,
            previous: None,
            sample_clocks: SampleClockHistory::default(),
            stage_times: StageTimeHistory::default(),
            control_lineage: ControlLineageHistory::default(),
            validated_samples: 0,
        })
    }

    /// Gets the number of samples that this validator accepted.
    #[must_use]
    pub const fn validated_samples(&self) -> u64 {
        self.validated_samples
    }

    /// Validates and records the next sample in one complete trace.
    pub fn validate_next(&mut self, sample: &TrialSample) -> Result<(), CodecError> {
        if let Some(previous) = &self.previous {
            sample.validate_adjacent_only(previous, self.run)?;
        } else {
            sample.validate_for_run(self.run)?;
        }
        let mut sample_clocks = self.sample_clocks.clone();
        let mut stage_times = self.stage_times.clone();
        let mut control_lineage = self.control_lineage.clone();
        sample_clocks.validate_and_record(sample)?;
        stage_times.validate_and_record(sample)?;
        control_lineage.validate_and_record(sample, self.run)?;
        self.sample_clocks = sample_clocks;
        self.stage_times = stage_times;
        self.control_lineage = control_lineage;
        self.previous = Some(sample.clone());
        self.validated_samples = self.validated_samples.wrapping_add(1);
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct SampleClockHistory {
    device: ClockTraceState,
    client: ClockTraceState,
    adapter: ClockTraceState,
    flight_controller: ClockTraceState,
    simulator: ClockTraceState,
}

impl SampleClockHistory {
    fn validate_and_record(&mut self, sample: &TrialSample) -> Result<(), ValidationError> {
        self.device.observe(
            "sample.time.device",
            ClockDomain::Device,
            sample.time.source_reading(ClockDomain::Device).copied(),
            sample.time.has_discontinuity(ClockDomain::Device),
        )?;
        self.client.observe(
            "sample.time.client",
            ClockDomain::Client,
            sample.time.source_reading(ClockDomain::Client).copied(),
            sample.time.has_discontinuity(ClockDomain::Client),
        )?;
        self.adapter.observe(
            "sample.time.adapter",
            ClockDomain::Adapter,
            sample.time.source_reading(ClockDomain::Adapter).copied(),
            sample.time.has_discontinuity(ClockDomain::Adapter),
        )?;
        self.flight_controller.observe(
            "sample.time.flight_controller",
            ClockDomain::FlightController,
            sample
                .time
                .source_reading(ClockDomain::FlightController)
                .copied(),
            sample.time.has_discontinuity(ClockDomain::FlightController),
        )?;
        self.simulator.observe(
            "sample.time.simulator",
            ClockDomain::Simulator,
            sample.time.source_reading(ClockDomain::Simulator).copied(),
            sample.time.has_discontinuity(ClockDomain::Simulator),
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ClockTraceState {
    last_present: Option<ClockReading>,
    discontinuity_pending: bool,
}

impl ClockTraceState {
    fn observe(
        &mut self,
        field: &str,
        domain: ClockDomain,
        current: Option<ClockReading>,
        has_discontinuity: bool,
    ) -> Result<(), ValidationError> {
        let Some(current) = current else {
            if has_discontinuity && self.discontinuity_pending {
                return invalid_clock(field, "a second discontinuity has no intervening reading");
            }
            self.discontinuity_pending |= has_discontinuity;
            return Ok(());
        };
        if let Some(previous) = self.last_present {
            self.validate_present(
                field,
                domain,
                previous,
                current,
                has_discontinuity || self.discontinuity_pending,
            )?;
        }
        self.last_present = Some(current);
        self.discontinuity_pending = false;
        Ok(())
    }

    fn validate_present(
        &self,
        field: &str,
        domain: ClockDomain,
        previous: ClockReading,
        current: ClockReading,
        has_discontinuity: bool,
    ) -> Result<(), ValidationError> {
        if previous.epoch == current.epoch && current.time_ns < previous.time_ns {
            return Err(ValidationError::ClockRegression {
                clock: format!("{domain:?}"),
                previous_ns: previous.time_ns,
                current_ns: current.time_ns,
            });
        }
        if previous.epoch == current.epoch && has_discontinuity {
            return invalid_clock(field, "a clock discontinuity must change the source epoch");
        }
        if previous.epoch != current.epoch && !has_discontinuity {
            return invalid_clock(field, "a source epoch change needs a clock discontinuity");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct StageTimeHistory {
    raw_input: StageTimeState,
    normalized_control: StageTimeState,
    typed_intent: StageTimeState,
    adapter_demand: StageTimeState,
    transmitted_setpoint: StageTimeState,
    flight_controller_estimate: StageTimeState,
    simulator_truth: StageTimeState,
}

impl StageTimeHistory {
    fn validate_and_record(&mut self, sample: &TrialSample) -> Result<(), ValidationError> {
        self.raw_input
            .observe("sample.raw_input", &sample.raw_input.stamp.source)?;
        self.normalized_control.observe(
            "sample.normalized_control",
            &sample.normalized_control.stamp.source,
        )?;
        self.typed_intent
            .observe("sample.typed_intent", &sample.typed_intent.stamp.source)?;
        self.adapter_demand
            .observe("sample.adapter_demand", &sample.adapter_demand.stamp.source)?;
        self.transmitted_setpoint.observe(
            "sample.transmitted_setpoint",
            &sample.transmitted_setpoint.stamp.source,
        )?;
        self.flight_controller_estimate.observe(
            "sample.flight_controller_estimate",
            &sample.flight_controller_estimate.stamp.source,
        )?;
        self.simulator_truth.observe(
            "sample.simulator_truth",
            &sample.simulator_truth.stamp.source,
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct StageTimeState {
    last_present: Option<StageTimePoint>,
}

impl StageTimeState {
    fn observe(&mut self, field: &str, source: &SourceStamp) -> Result<(), ValidationError> {
        let Some(time_ns) = source.time_ns.value().copied() else {
            return Ok(());
        };
        let current = StageTimePoint {
            clock: source.clock,
            epoch: source.epoch,
            time_ns,
        };
        if let Some(previous) = self.last_present
            && previous.clock == current.clock
            && previous.epoch == current.epoch
            && current.time_ns < previous.time_ns
        {
            return invalid_stage(field, "the source time moves back after a missing record");
        }
        self.last_present = Some(current);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct StageTimePoint {
    clock: ClockDomain,
    epoch: u64,
    time_ns: u64,
}

#[derive(Clone, Debug, Default)]
struct ControlLineageHistory {
    raw_input: EventHistory,
    normalized_control: EventHistory,
    typed_intent: EventHistory,
    adapter_demand: EventHistory,
}

impl ControlLineageHistory {
    fn validate_and_record(
        &mut self,
        sample: &TrialSample,
        run: &RunIdentity,
    ) -> Result<(), ValidationError> {
        self.raw_input.record(
            ControlStage::RawInput,
            &sample.raw_input,
            "sample.raw_input",
        )?;
        self.raw_input
            .consume(&sample.normalized_control, "sample.normalized_control", run)?;
        self.raw_input.validate_limit("sample.normalized_control")?;
        self.normalized_control.record(
            ControlStage::NormalizedControl,
            &sample.normalized_control,
            "sample.normalized_control",
        )?;
        self.normalized_control
            .consume(&sample.typed_intent, "sample.typed_intent", run)?;
        self.normalized_control
            .validate_limit("sample.typed_intent")?;
        self.typed_intent.record(
            ControlStage::TypedIntent,
            &sample.typed_intent,
            "sample.typed_intent",
        )?;
        self.typed_intent
            .consume(&sample.adapter_demand, "sample.adapter_demand", run)?;
        self.typed_intent.validate_limit("sample.adapter_demand")?;
        self.adapter_demand.record(
            ControlStage::AdapterDemand,
            &sample.adapter_demand,
            "sample.adapter_demand",
        )?;
        self.adapter_demand.consume(
            &sample.transmitted_setpoint,
            "sample.transmitted_setpoint",
            run,
        )?;
        self.adapter_demand
            .validate_limit("sample.transmitted_setpoint")
    }
}

#[derive(Clone, Debug, Default)]
struct EventHistory {
    events: VecDeque<EventRecord>,
}

impl EventHistory {
    fn record<T>(
        &mut self,
        stage: ControlStage,
        event: &CausalStage<T>,
        field: &str,
    ) -> Result<(), ValidationError> {
        if event.value().is_none() {
            return Ok(());
        }
        let id = event.stamp.control_event_id(stage);
        if let Some(existing) = self.events.iter().find(|record| record.id == id) {
            return if existing.stamp == event.stamp {
                Ok(())
            } else {
                invalid_stage(field, "one control event has different stamps")
            };
        }
        self.events.push_back(EventRecord {
            id,
            stamp: event.stamp.clone(),
        });
        Ok(())
    }

    fn consume<T>(
        &mut self,
        current: &CausalStage<T>,
        field: &str,
        run: &RunIdentity,
    ) -> Result<(), ValidationError> {
        if current.value().is_none() {
            return Ok(());
        }
        let Some(predecessor_id) = current.stamp.predecessor else {
            return invalid_stage(field, "the control event has no predecessor identity");
        };
        let Some(position) = self
            .events
            .iter()
            .position(|record| record.id == predecessor_id)
        else {
            return Err(ValidationError::UnknownControlPredecessor {
                field: field.to_owned(),
                stage: format!("{:?}", predecessor_id.stage),
                clock: format!("{:?}", predecessor_id.clock),
                epoch: predecessor_id.epoch,
                sequence: predecessor_id.sequence,
            });
        };
        let predecessor = self.events[position].stamp.clone();
        validate_event_order(&predecessor, &current.stamp, field, run)?;
        let retained = self.events.split_off(position);
        self.events = retained;
        Ok(())
    }

    fn validate_limit(&self, field: &str) -> Result<(), ValidationError> {
        if self.events.len() <= MAX_CONTROL_EVENT_HISTORY {
            return Ok(());
        }
        Err(ValidationError::TooManyItems {
            field: format!("{field}.unconsumed_predecessors"),
            count: self.events.len(),
            limit: MAX_CONTROL_EVENT_HISTORY,
        })
    }
}

#[derive(Clone, Debug)]
struct EventRecord {
    id: ControlEventId,
    stamp: StageStamp,
}

fn validate_event_order(
    predecessor: &StageStamp,
    current: &StageStamp,
    field: &str,
    run: &RunIdentity,
) -> Result<(), ValidationError> {
    let Some(predecessor_ns) = predecessor.source.time_ns.value().copied() else {
        return Ok(());
    };
    let Some(current_ns) = current.source.time_ns.value().copied() else {
        return Ok(());
    };
    let predecessor_time = run.map_clock_time(
        field,
        predecessor.source.clock,
        predecessor.source.epoch,
        predecessor_ns,
    )?;
    let current_time = run.map_clock_time(
        field,
        current.source.clock,
        current.source.epoch,
        current_ns,
    )?;
    if current_time.latest_ns < predecessor_time.earliest_ns {
        return Err(ValidationError::CausalMappedSourceOrder {
            field: field.to_owned(),
            predecessor_earliest_ns: predecessor_time.earliest_ns,
            current_latest_ns: current_time.latest_ns,
        });
    }
    Ok(())
}

fn invalid_clock<T>(field: &str, reason: &'static str) -> Result<T, ValidationError> {
    Err(ValidationError::InvalidClockObservation {
        field: field.to_owned(),
        reason,
    })
}

fn invalid_stage<T>(field: &str, reason: &'static str) -> Result<T, ValidationError> {
    Err(ValidationError::InvalidStageStamp {
        field: field.to_owned(),
        reason,
    })
}

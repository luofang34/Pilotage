//! What one complete sample stream counted, by factor and by lane.
//!
//! The counts are the terminal statement of a run: how many samples each
//! factor could have changed, how many it did change, how many it held, how
//! many bypassed every policy, and how many the plant constrained. They are
//! folded from the verified stream and never read back from the executor,
//! so a count can only say what the samples say.

use serde::{Deserialize, Serialize};

use super::super::invalid_terminal;
use super::EXECUTED_UNCERTAINTY_SCHEMA_VERSION;
use super::sample::{ExecutedBypassReason, ExecutedSample};
use crate::TuneError;

/// What one declared sensor lane did across a complete stream.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedSensorLaneCounts {
    /// The one-byte lane tag this count answers for.
    pub lane_tag: u8,
    /// Samples in which the lane was present and could be changed.
    pub eligible: u64,
    /// Samples in which the exact lane value moved.
    pub changed: u64,
    /// Samples that reused an offset an earlier sample drew.
    pub held: u64,
}

/// How many samples each bypass reason answered for.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedBypassCounts {
    /// The controller supplied no actuator answer.
    pub missing_answer: u64,
    /// The answer named a different number of lanes.
    pub invalid_actuator_count: u64,
    /// A backup path produced the command.
    pub backup: u64,
    /// A direct command reached the actuator.
    pub direct: u64,
    /// A failsafe produced the command.
    pub failsafe: u64,
    /// The controller raised a fallback lane.
    pub fallback_mask: u64,
    /// The armed state changed on the sample.
    pub arm_transition: u64,
    /// The vehicle was not armed.
    pub disarmed: u64,
    /// An emergency termination produced the command.
    pub emergency_termination: u64,
}

impl ExecutedBypassCounts {
    /// Returns the total number of bypassed samples.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.missing_answer
            .wrapping_add(self.invalid_actuator_count)
            .wrapping_add(self.backup)
            .wrapping_add(self.direct)
            .wrapping_add(self.failsafe)
            .wrapping_add(self.fallback_mask)
            .wrapping_add(self.arm_transition)
            .wrapping_add(self.disarmed)
            .wrapping_add(self.emergency_termination)
    }

    fn count(&mut self, reason: ExecutedBypassReason) {
        let slot = match reason {
            ExecutedBypassReason::MissingAnswer => &mut self.missing_answer,
            ExecutedBypassReason::InvalidActuatorCount => &mut self.invalid_actuator_count,
            ExecutedBypassReason::Backup => &mut self.backup,
            ExecutedBypassReason::Direct => &mut self.direct,
            ExecutedBypassReason::Failsafe => &mut self.failsafe,
            ExecutedBypassReason::FallbackMask => &mut self.fallback_mask,
            ExecutedBypassReason::ArmTransition => &mut self.arm_transition,
            ExecutedBypassReason::Disarmed => &mut self.disarmed,
            ExecutedBypassReason::EmergencyTermination => &mut self.emergency_termination,
        };
        *slot = slot.wrapping_add(1);
    }
}

/// What the actuator factors did across a complete stream.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedActuatorCounts {
    /// Samples that carried actuator evidence.
    pub commanded: u64,
    /// Samples eligible for an actuator perturbation.
    pub eligible: u64,
    /// Samples that only primed the accepted-command history.
    pub primed: u64,
    /// Samples the seeded schedule selected for a hold.
    pub selected_hold: u64,
    /// Samples on which a hold was applied.
    pub applied_hold: u64,
    /// Samples the authority scale changed.
    pub scaled: u64,
    /// Samples a plant constraint changed.
    pub clamped: u64,
    /// Samples that bypassed every perturbation policy.
    pub bypassed: ExecutedBypassCounts,
}

/// The terminal counts one complete sample stream states.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutedUncertaintyLedger {
    /// Ledger schema version.
    pub schema_version: u16,
    /// The number of samples the stream carried.
    pub sample_count: u64,
    /// The first global sample sequence the stream carried.
    pub first_global_sample_sequence: u64,
    /// The last global sample sequence the stream carried.
    pub last_global_sample_sequence: u64,
    /// The per-lane sensor counts, in ascending lane-tag order.
    pub sensor_lanes: Vec<ExecutedSensorLaneCounts>,
    /// The actuator counts.
    pub actuator: ExecutedActuatorCounts,
}

impl ExecutedUncertaintyLedger {
    /// Opens an empty ledger for the declared lanes.
    #[must_use]
    pub fn opened(lane_tags: &[u8]) -> Self {
        Self {
            schema_version: EXECUTED_UNCERTAINTY_SCHEMA_VERSION,
            sample_count: 0,
            first_global_sample_sequence: 0,
            last_global_sample_sequence: 0,
            sensor_lanes: lane_tags
                .iter()
                .map(|lane_tag| ExecutedSensorLaneCounts {
                    lane_tag: *lane_tag,
                    ..ExecutedSensorLaneCounts::default()
                })
                .collect(),
            actuator: ExecutedActuatorCounts::default(),
        }
    }

    /// Counts one verified sample.
    pub fn count(&mut self, sample: &ExecutedSample, drawn: &[bool], scaled: bool) {
        if self.sample_count == 0 {
            self.first_global_sample_sequence = sample.global_sample_sequence;
        }
        self.sample_count = self.sample_count.wrapping_add(1);
        self.last_global_sample_sequence = sample.global_sample_sequence;
        self.count_sensor(sample, drawn);
        self.count_actuator(sample, scaled);
    }

    /// Rejects a ledger that no complete stream could have produced.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the schema differs, when the lane order
    /// is not the declared one, when the sequence span cannot hold the
    /// counted samples, or when an actuator total exceeds its population.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != EXECUTED_UNCERTAINTY_SCHEMA_VERSION {
            return Err(invalid_terminal(
                "the executed uncertainty ledger schema changed",
            ));
        }
        self.validate_span()?;
        let mut previous: Option<u8> = None;
        for lane in &self.sensor_lanes {
            if previous.is_some_and(|prior| prior >= lane.lane_tag) {
                return Err(invalid_terminal("the ledger lanes are not in lane order"));
            }
            previous = Some(lane.lane_tag);
            if lane.changed > lane.eligible || lane.held > lane.eligible {
                return Err(invalid_terminal("a lane counted more changes than samples"));
            }
        }
        self.validate_actuator()
    }

    fn validate_span(&self) -> Result<(), TuneError> {
        if self.sample_count == 0 {
            return Err(invalid_terminal("an executed uncertainty stream is empty"));
        }
        let span = self
            .last_global_sample_sequence
            .checked_sub(self.first_global_sample_sequence)
            .and_then(|span| span.checked_add(1))
            .ok_or_else(|| invalid_terminal("the ledger sample sequence span rewinds"))?;
        if span != self.sample_count {
            return Err(invalid_terminal(
                "the ledger sample count does not span its sequence",
            ));
        }
        Ok(())
    }

    fn validate_actuator(&self) -> Result<(), TuneError> {
        let actuator = self.actuator;
        if actuator.commanded > self.sample_count
            || actuator.eligible.wrapping_add(actuator.bypassed.total()) != actuator.commanded
            || actuator.applied_hold > actuator.selected_hold
            || actuator.selected_hold > actuator.eligible
            || actuator.primed > actuator.eligible
            || actuator.scaled > actuator.eligible
            || actuator.clamped > self.sample_count
        {
            return Err(invalid_terminal(
                "the ledger actuator counts do not answer for their samples",
            ));
        }
        Ok(())
    }

    fn count_sensor(&mut self, sample: &ExecutedSample, drawn: &[bool]) {
        let Some(sensor) = sample.sensor else {
            return;
        };
        for (index, lane) in self.sensor_lanes.iter_mut().enumerate() {
            let bit = 1_u16 << lane.lane_tag;
            if sensor.presence_mask & bit == 0 {
                continue;
            }
            lane.eligible = lane.eligible.wrapping_add(1);
            if sensor.changed_mask & bit != 0 {
                lane.changed = lane.changed.wrapping_add(1);
            }
            if !drawn.get(index).copied().unwrap_or(true) {
                lane.held = lane.held.wrapping_add(1);
            }
        }
    }

    fn count_actuator(&mut self, sample: &ExecutedSample, scaled: bool) {
        let Some(actuator) = sample.actuator else {
            return;
        };
        let counts = &mut self.actuator;
        counts.commanded = counts.commanded.wrapping_add(1);
        if sample.constraints.clamped() {
            counts.clamped = counts.clamped.wrapping_add(1);
        }
        match actuator.eligibility {
            super::sample::ExecutedEligibility::Bypass(reason) => counts.bypassed.count(reason),
            super::sample::ExecutedEligibility::Eligible => {
                counts.eligible = counts.eligible.wrapping_add(1);
                if actuator.prime {
                    counts.primed = counts.primed.wrapping_add(1);
                }
                if actuator.selected_hold {
                    counts.selected_hold = counts.selected_hold.wrapping_add(1);
                }
                if actuator.applied_hold {
                    counts.applied_hold = counts.applied_hold.wrapping_add(1);
                }
                if scaled {
                    counts.scaled = counts.scaled.wrapping_add(1);
                }
            }
        }
    }
}

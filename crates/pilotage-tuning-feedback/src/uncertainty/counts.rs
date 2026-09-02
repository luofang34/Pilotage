//! The counts this verifier folds for itself from the verified samples.
//!
//! Nothing is read out of the ledger the run states. Every count is built
//! again from the samples, and the stated ledger is then required to be
//! exactly that, so a run cannot state a count no sample produced.

use flight_tune::{
    ExecutedActuatorCounts, ExecutedBypassCounts, ExecutedBypassReason, ExecutedEligibility,
    ExecutedSample, ExecutedSensorLaneCounts, ExecutedUncertaintyLedger,
};

/// The counts one walked stream produces.
pub(super) struct DerivedCounts {
    sample_count: u64,
    first_global_sample_sequence: u64,
    last_global_sample_sequence: u64,
    sensor_lanes: Vec<ExecutedSensorLaneCounts>,
    actuator: ExecutedActuatorCounts,
}

impl DerivedCounts {
    /// Opens empty counts for the declared lanes.
    pub(super) fn opened(lane_tags: &[u8]) -> Self {
        Self {
            sample_count: 0,
            first_global_sample_sequence: 0,
            last_global_sample_sequence: 0,
            sensor_lanes: lane_tags
                .iter()
                .map(|lane_tag| ExecutedSensorLaneCounts {
                    lane_tag: *lane_tag,
                    eligible: 0,
                    changed: 0,
                    held: 0,
                })
                .collect(),
            actuator: ExecutedActuatorCounts {
                commanded: 0,
                eligible: 0,
                primed: 0,
                selected_hold: 0,
                applied_hold: 0,
                scaled: 0,
                clamped: 0,
                bypassed: ExecutedBypassCounts {
                    missing_answer: 0,
                    invalid_actuator_count: 0,
                    backup: 0,
                    direct: 0,
                    failsafe: 0,
                    fallback_mask: 0,
                    arm_transition: 0,
                    disarmed: 0,
                    emergency_termination: 0,
                },
            },
        }
    }

    /// Counts one sample the relation accepted.
    pub(super) fn count(&mut self, sample: &ExecutedSample, drawn: &[bool], scaled: bool) {
        if self.sample_count == 0 {
            self.first_global_sample_sequence = sample.global_sample_sequence;
        }
        self.sample_count = self.sample_count.wrapping_add(1);
        self.last_global_sample_sequence = sample.global_sample_sequence;
        self.count_sensor(sample, drawn);
        self.count_actuator(sample, scaled);
    }

    /// States the ledger these samples produced.
    pub(super) fn stated(self, schema_version: u16) -> ExecutedUncertaintyLedger {
        ExecutedUncertaintyLedger {
            schema_version,
            sample_count: self.sample_count,
            first_global_sample_sequence: self.first_global_sample_sequence,
            last_global_sample_sequence: self.last_global_sample_sequence,
            sensor_lanes: self.sensor_lanes,
            actuator: self.actuator,
        }
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
            ExecutedEligibility::Bypass(reason) => count_bypass(&mut counts.bypassed, reason),
            ExecutedEligibility::Eligible => {
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

fn count_bypass(counts: &mut ExecutedBypassCounts, reason: ExecutedBypassReason) {
    let slot = match reason {
        ExecutedBypassReason::MissingAnswer => &mut counts.missing_answer,
        ExecutedBypassReason::InvalidActuatorCount => &mut counts.invalid_actuator_count,
        ExecutedBypassReason::Backup => &mut counts.backup,
        ExecutedBypassReason::Direct => &mut counts.direct,
        ExecutedBypassReason::Failsafe => &mut counts.failsafe,
        ExecutedBypassReason::FallbackMask => &mut counts.fallback_mask,
        ExecutedBypassReason::ArmTransition => &mut counts.arm_transition,
        ExecutedBypassReason::Disarmed => &mut counts.disarmed,
        ExecutedBypassReason::EmergencyTermination => &mut counts.emergency_termination,
    };
    *slot = slot.wrapping_add(1);
}

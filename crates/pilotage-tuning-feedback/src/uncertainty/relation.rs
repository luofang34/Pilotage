//! Walking one sample stream and deriving every decision it states.
//!
//! The walk is the whole relation restated: sequence discipline, the lane
//! offsets, both sensor identities, the authority scale, the seeded hold
//! schedule, the interval identity, and the hover force. A stated value that
//! this walk does not derive ends the verification.

use flight_tune::{
    Digest, ExecutedHoverInitialization, ExecutedSample, ExecutedSensorApplication,
    ExecutedUncertaintyDeclaration, ExecutedUncertaintyLedger,
};

use super::actuator::ActuatorState;
use super::counts::DerivedCounts;
use super::derivation::{self, SENSOR_LANE_COUNT};
use crate::{FeedbackError, error::invalid};

/// Derives the ledger one declaration and one ordered stream produce.
///
/// # Errors
///
/// Returns [`FeedbackError`] when the stream repeats, skips, or rewinds a
/// sequence, when a sample has no completed lockstep answer, when the hover
/// initialization moves, or when any stated value is not the derived one.
pub(super) fn walk(
    declaration: &ExecutedUncertaintyDeclaration,
    samples: &[ExecutedSample],
    schema_version: u16,
) -> Result<ExecutedUncertaintyLedger, FeedbackError> {
    let lane_tags = declaration
        .sensor_lanes
        .iter()
        .map(|lane| lane.lane_tag)
        .collect::<Vec<_>>();
    let mut walk = Walk {
        counts: DerivedCounts::opened(&lane_tags),
        actuator: ActuatorState::new(),
        drawn_buckets: [None; SENSOR_LANE_COUNT],
        hover: None,
        last_sequence: None,
        last_global_sample_sequence: None,
        last_timestamp_us: None,
    };
    for sample in samples {
        walk.accept(declaration, sample)?;
    }
    if walk.actuator.holds_a_command() {
        return Err(invalid("the stream ended with an active command hold"));
    }
    Ok(walk.counts.stated(schema_version))
}

struct Walk {
    counts: DerivedCounts,
    actuator: ActuatorState,
    drawn_buckets: [Option<u64>; SENSOR_LANE_COUNT],
    hover: Option<ExecutedHoverInitialization>,
    last_sequence: Option<u64>,
    last_global_sample_sequence: Option<u64>,
    last_timestamp_us: Option<u64>,
}

impl Walk {
    fn accept(
        &mut self,
        declaration: &ExecutedUncertaintyDeclaration,
        sample: &ExecutedSample,
    ) -> Result<(), FeedbackError> {
        self.advance(sample)?;
        self.require_hover(declaration, sample)?;
        require_send(sample)?;
        let drawn = self.sensor(declaration, sample)?;
        let scaled = self.actuator.accept(declaration, sample)?;
        self.counts.count(sample, &drawn, scaled);
        Ok(())
    }

    fn advance(&mut self, sample: &ExecutedSample) -> Result<(), FeedbackError> {
        require_step(self.last_sequence, sample.sequence, "trace sequence")?;
        require_step(
            self.last_global_sample_sequence,
            sample.global_sample_sequence,
            "sample sequence",
        )?;
        if self
            .last_timestamp_us
            .is_some_and(|previous| sample.simulator_timestamp_us < previous)
        {
            return Err(invalid("a sample rewinds the simulation time"));
        }
        self.last_sequence = Some(sample.sequence);
        self.last_global_sample_sequence = Some(sample.global_sample_sequence);
        self.last_timestamp_us = Some(sample.simulator_timestamp_us);
        Ok(())
    }

    fn require_hover(
        &mut self,
        declaration: &ExecutedUncertaintyDeclaration,
        sample: &ExecutedSample,
    ) -> Result<(), FeedbackError> {
        let hover = sample.hover;
        if hover.scale_basis_points != declaration.hover_scale_basis_points
            || !hover.estimator_disabled
        {
            return Err(invalid(
                "a sample does not state the declared hover initialization",
            ));
        }
        if derivation::scaled_hover_force(hover.baseline_force_bits, hover.scale_basis_points)
            != hover.effective_force_bits
        {
            return Err(invalid(
                "the hover force does not follow from its baseline and scale",
            ));
        }
        match self.hover {
            Some(first) if first != hover => {
                Err(invalid("the hover initialization changed inside one run"))
            }
            Some(_) => Ok(()),
            None => {
                self.hover = Some(hover);
                Ok(())
            }
        }
    }

    fn sensor(
        &mut self,
        declaration: &ExecutedUncertaintyDeclaration,
        sample: &ExecutedSample,
    ) -> Result<Vec<bool>, FeedbackError> {
        let Some(sensor) = sample.sensor else {
            if !declaration.sensor_lanes.is_empty() && sample.armed {
                return Err(invalid(
                    "an armed sample carries no sensor evidence for a declared lane",
                ));
            }
            return Ok(vec![false; declaration.sensor_lanes.len()]);
        };
        require_identity(&sensor)?;
        require_undeclared_unchanged(declaration, &sensor)?;
        let mut drawn = Vec::with_capacity(declaration.sensor_lanes.len());
        for declared in &declaration.sensor_lanes {
            drawn.push(self.lane(
                declaration,
                sample,
                &sensor,
                declared.lane_tag,
                declared.peak_amplitude_bits,
                declared.update_interval_samples,
            )?);
        }
        Ok(drawn)
    }

    fn lane(
        &mut self,
        declaration: &ExecutedUncertaintyDeclaration,
        sample: &ExecutedSample,
        sensor: &ExecutedSensorApplication,
        lane_tag: u8,
        peak_amplitude_bits: u32,
        update_interval_samples: u32,
    ) -> Result<bool, FeedbackError> {
        let lane = usize::from(lane_tag);
        let bit = 1_u16 << lane_tag;
        if sensor.presence_mask & bit == 0 {
            if sensor.update_buckets[lane].is_some() {
                return Err(invalid("an absent sensor lane drew an offset"));
            }
            return Ok(false);
        }
        if update_interval_samples == 0 {
            return Err(invalid("a declared sensor lane holds no offset"));
        }
        let bucket = sample.global_sample_sequence / u64::from(update_interval_samples);
        if sensor.update_buckets[lane] != Some(bucket) {
            return Err(invalid(
                "a sensor lane states another held-offset bucket than the declared one",
            ));
        }
        let raw = sensor.raw_value_bits[lane]
            .ok_or_else(|| invalid("a present sensor lane carries no value"))?;
        let value = f32::from_bits(raw);
        if !value.is_finite() {
            return Err(invalid("a declared sensor lane carries no value"));
        }
        let offset = derivation::sensor_offset(
            declaration.condition_digest,
            declaration.run_seed,
            lane_tag,
            bucket,
            f32::from_bits(peak_amplitude_bits),
        );
        let required = (value + offset).to_bits();
        if sensor.effective_value_bits[lane] != Some(required)
            || (sensor.changed_mask & bit != 0) != (required != raw)
        {
            return Err(invalid("a sensor lane does not carry its declared value"));
        }
        let drew = self.drawn_buckets[lane] != Some(bucket);
        self.drawn_buckets[lane] = Some(bucket);
        Ok(drew)
    }
}

/// Requires both sensor identities to cover the values they travel with.
fn require_identity(sensor: &ExecutedSensorApplication) -> Result<(), FeedbackError> {
    if derivation::sensor_sample_digest(sensor.presence_mask, &sensor.raw_value_bits)
        != sensor.raw_digest
    {
        return Err(invalid(
            "the raw sensor identity does not cover its own values",
        ));
    }
    if derivation::sensor_sample_digest(sensor.presence_mask, &sensor.effective_value_bits)
        != sensor.effective_digest
    {
        return Err(invalid(
            "the effective sensor identity does not cover its own values",
        ));
    }
    Ok(())
}

/// Requires a lane the declaration never named to reach the controller whole.
fn require_undeclared_unchanged(
    declaration: &ExecutedUncertaintyDeclaration,
    sensor: &ExecutedSensorApplication,
) -> Result<(), FeedbackError> {
    for lane in 0..SENSOR_LANE_COUNT {
        let tag = u8::try_from(lane).map_err(|_| invalid("a sensor lane is not addressable"))?;
        if declaration.lane(tag).is_some() {
            continue;
        }
        if sensor.update_buckets[lane].is_some() {
            return Err(invalid("an undeclared sensor lane drew an offset"));
        }
        if sensor.raw_value_bits[lane] != sensor.effective_value_bits[lane]
            || sensor.changed_mask & (1 << lane) != 0
        {
            return Err(invalid("an undeclared sensor lane changed"));
        }
    }
    Ok(())
}

/// Requires one sample to carry exactly one completed lockstep answer.
fn require_send(sample: &ExecutedSample) -> Result<(), FeedbackError> {
    let send = sample.send;
    if !send.attempted || !send.succeeded || !send.lockstep {
        return Err(invalid("a sample has no completed lockstep actuator send"));
    }
    if send.echoed_timestamp_us != sample.simulator_timestamp_us {
        return Err(invalid("a sample send answered another sensor sample"));
    }
    if sample.constraints.trace_failure {
        return Err(invalid("a sample reports its own trace failure"));
    }
    Ok(())
}

/// Requires one counter to advance by exactly one step.
fn require_step(previous: Option<u64>, current: u64, name: &str) -> Result<(), FeedbackError> {
    let Some(previous) = previous else {
        return Ok(());
    };
    if current == previous {
        return Err(invalid(format!("a sample repeats one {name}")));
    }
    if current < previous {
        return Err(invalid(format!("a sample rewinds the {name}")));
    }
    if current.wrapping_sub(previous) != 1 {
        return Err(invalid(format!("a sample skips a {name}")));
    }
    Ok(())
}

/// Requires one derived identity to equal the stated one.
pub(super) fn require_digest(
    derived: Digest,
    stated: Digest,
    detail: &'static str,
) -> Result<(), FeedbackError> {
    if derived == stated {
        return Ok(());
    }
    Err(invalid(detail))
}

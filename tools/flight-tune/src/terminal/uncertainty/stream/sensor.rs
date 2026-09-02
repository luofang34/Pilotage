//! Deriving what one declaration required of each sensor lane.
//!
//! A declared lane holds one drawn offset for a complete update interval, so
//! the required effective value follows from the raw value the sample states
//! and nothing else. A lane the declaration never named must arrive at the
//! controller unchanged, which is what keeps a sensor request out of the
//! simulator truth evidence stream.

use super::super::super::invalid_terminal;
use super::super::sample::{ExecutedSample, ExecutedSensorApplication};
use super::super::{
    DeclaredSensorLane, EXECUTED_SENSOR_LANE_COUNT, ExecutedUncertaintyDeclaration, derivation,
};
use super::require_digest;
use crate::TuneError;

/// The update bucket each declared lane last drew an offset for.
pub(super) struct SensorState {
    drawn_buckets: [Option<u64>; EXECUTED_SENSOR_LANE_COUNT],
}

impl SensorState {
    pub(super) const fn new() -> Self {
        Self {
            drawn_buckets: [None; EXECUTED_SENSOR_LANE_COUNT],
        }
    }
}

/// Derives every sensor decision one sample states.
///
/// Returns, for each declared lane in declaration order, whether this sample
/// drew a new offset rather than holding one an earlier sample drew.
pub(super) fn verify(
    state: &mut SensorState,
    declaration: &ExecutedUncertaintyDeclaration,
    sample: &ExecutedSample,
) -> Result<Vec<bool>, TuneError> {
    let Some(sensor) = sample.sensor else {
        if !declaration.sensor_lanes.is_empty() && sample.armed {
            return Err(invalid_terminal(
                "an armed sample carries no sensor evidence for a declared lane",
            ));
        }
        return Ok(vec![false; declaration.sensor_lanes.len()]);
    };
    require_digest(
        derivation::sensor_sample_digest(sensor.presence_mask, &sensor.raw_value_bits),
        sensor.raw_digest,
        "the raw sensor identity does not cover its own values",
    )?;
    require_digest(
        derivation::sensor_sample_digest(sensor.presence_mask, &sensor.effective_value_bits),
        sensor.effective_digest,
        "the effective sensor identity does not cover its own values",
    )?;
    require_undeclared_unchanged(declaration, &sensor)?;
    declaration
        .sensor_lanes
        .iter()
        .map(|declared| verify_lane(state, declaration, sample, &sensor, *declared))
        .collect()
}

/// Requires a lane the declaration never named to reach the controller whole.
fn require_undeclared_unchanged(
    declaration: &ExecutedUncertaintyDeclaration,
    sensor: &ExecutedSensorApplication,
) -> Result<(), TuneError> {
    for lane in 0..EXECUTED_SENSOR_LANE_COUNT {
        let tag = u8::try_from(lane)
            .map_err(|_| invalid_terminal("a sensor lane tag is not addressable"))?;
        if declaration.lane(tag).is_some() {
            continue;
        }
        if sensor.update_buckets[lane].is_some() {
            return Err(invalid_terminal("an undeclared sensor lane drew an offset"));
        }
        if sensor.raw_value_bits[lane] != sensor.effective_value_bits[lane]
            || sensor.changed_mask & (1 << lane) != 0
        {
            return Err(invalid_terminal("an undeclared sensor lane changed"));
        }
    }
    Ok(())
}

/// Derives the one value a declared lane must carry, and states whether it
/// drew a new offset for this sample.
fn verify_lane(
    state: &mut SensorState,
    declaration: &ExecutedUncertaintyDeclaration,
    sample: &ExecutedSample,
    sensor: &ExecutedSensorApplication,
    declared: DeclaredSensorLane,
) -> Result<bool, TuneError> {
    let lane = usize::from(declared.lane_tag);
    let bit = 1_u16 << declared.lane_tag;
    let present = sensor.presence_mask & bit != 0;
    let stated_bucket = sensor.update_buckets[lane];
    if !present {
        if stated_bucket.is_some() {
            return Err(invalid_terminal("an absent sensor lane drew an offset"));
        }
        return Ok(false);
    }
    let update_bucket = sample.global_sample_sequence / u64::from(declared.update_interval_samples);
    if stated_bucket != Some(update_bucket) {
        return Err(invalid_terminal(
            "a sensor lane states another held-offset bucket than the declared one",
        ));
    }
    let raw = sensor.raw_value_bits[lane]
        .ok_or_else(|| invalid_terminal("a present sensor lane carries no value"))?;
    require_effective(declaration, declared, update_bucket, raw, sensor, lane, bit)?;
    let drew = state.drawn_buckets[lane] != Some(update_bucket);
    state.drawn_buckets[lane] = Some(update_bucket);
    Ok(drew)
}

/// Requires one present declared lane to carry its derived value exactly.
#[allow(clippy::too_many_arguments)]
fn require_effective(
    declaration: &ExecutedUncertaintyDeclaration,
    declared: DeclaredSensorLane,
    update_bucket: u64,
    raw: u32,
    sensor: &ExecutedSensorApplication,
    lane: usize,
    bit: u16,
) -> Result<(), TuneError> {
    let value = f32::from_bits(raw);
    if !value.is_finite() {
        return Err(invalid_terminal("a declared sensor lane carries no value"));
    }
    let offset = derivation::sensor_offset(
        declaration.condition_digest,
        declaration.run_seed,
        declared.lane_tag,
        update_bucket,
        f32::from_bits(declared.peak_amplitude_bits),
    );
    let required = (value + offset).to_bits();
    if sensor.effective_value_bits[lane] != Some(required) {
        return Err(invalid_terminal(
            "a sensor lane does not carry its declared value",
        ));
    }
    if (sensor.changed_mask & bit != 0) != (required != raw) {
        return Err(invalid_terminal(
            "a sensor lane change flag does not match its own values",
        ));
    }
    Ok(())
}

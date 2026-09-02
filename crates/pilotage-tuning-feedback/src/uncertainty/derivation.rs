//! The seeded decisions this verifier derives for itself.
//!
//! Nothing here calls the core. Every domain string, every preimage order,
//! and every narrowing is written out again, so a change on the producing
//! side that this file does not also make is a difference the relation
//! reports rather than a change both sides agree to silently.

use flight_tune::Digest;
use sha2::{Digest as ShaDigest, Sha256};

use crate::{FeedbackError, error::invalid};

/// The domain the executing contract draws sensor noise under.
const SENSOR_NOISE_DOMAIN: &[u8] = b"pilotage-sensor-noise-v1";

/// The domain the executing contract draws command holds under.
const COMMAND_HOLD_DOMAIN: &[u8] = b"pilotage-command-hold-v1";

/// The domain the executing contract identifies a sensor sample under.
const EFFECTIVE_SENSOR_DOMAIN: &[u8] = b"aviate-effective-sensor-v1";

/// The nominal basis-point value, which requests no scaling.
const NOMINAL_BASIS_POINTS: u16 = 10_000;

/// The number of flight-controller sensor lanes the contract names.
pub(super) const SENSOR_LANE_COUNT: usize = 12;

/// Derives the held offset for one lane and one update bucket.
pub(super) fn sensor_offset(
    condition_digest: Digest,
    run_seed: u64,
    lane_tag: u8,
    update_bucket: u64,
    peak_amplitude: f32,
) -> f32 {
    let mut hasher = Sha256::new();
    hasher.update(SENSOR_NOISE_DOMAIN);
    hasher.update(condition_digest.as_bytes());
    hasher.update(run_seed.to_le_bytes());
    hasher.update([lane_tag]);
    hasher.update(update_bucket.to_le_bytes());
    let bytes = hasher.finalize();
    let sample = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let unit = f64::from(sample) / f64::from(u32::MAX);
    (unit.mul_add(2.0, -1.0) * f64::from(peak_amplitude)) as f32
}

/// Derives the identity of one complete command-hold interval.
pub(super) fn interval_identity(
    condition_digest: Digest,
    run_seed: u64,
    interval_epoch: u64,
    interval_index: u64,
    first_sequence: u64,
) -> Digest {
    let hasher = interval_hasher(
        condition_digest,
        run_seed,
        interval_epoch,
        interval_index,
        first_sequence,
    );
    Digest::from_bytes(hasher.finalize().into())
}

/// Derives the exact held positions of one complete decision interval.
///
/// # Errors
///
/// Returns [`FeedbackError`] when the declared interval cannot address its
/// own positions on this platform.
pub(super) fn hold_schedule(
    condition_digest: Digest,
    run_seed: u64,
    interval_epoch: u64,
    interval_index: u64,
    first_sequence: u64,
    fraction_basis_points: u16,
    decision_interval_samples: u32,
) -> Result<Vec<bool>, FeedbackError> {
    let size = usize::try_from(decision_interval_samples)
        .map_err(|_| invalid("a declared decision interval is not addressable"))?;
    let mut positions = (0..size).collect::<Vec<_>>();
    for cursor in (1..positions.len()).rev() {
        let encoded = u64::try_from(cursor)
            .map_err(|_| invalid("a decision interval position is not addressable"))?;
        let value = permutation_value(
            condition_digest,
            run_seed,
            interval_epoch,
            interval_index,
            first_sequence,
            encoded,
        );
        let swap = usize::try_from(value % encoded.wrapping_add(1))
            .map_err(|_| invalid("a decision interval swap is not addressable"))?;
        positions.swap(cursor, swap);
    }
    let count = u64::from(fraction_basis_points) * u64::from(decision_interval_samples)
        / u64::from(NOMINAL_BASIS_POINTS);
    let count =
        usize::try_from(count).map_err(|_| invalid("a declared hold count is not addressable"))?;
    let mut decisions = vec![false; size];
    for position in positions.into_iter().take(count) {
        decisions[position] = true;
    }
    Ok(decisions)
}

/// Derives the identity of one exact sensor sample.
pub(super) fn sensor_sample_digest(
    presence_mask: u16,
    values: &[Option<u32>; SENSOR_LANE_COUNT],
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(EFFECTIVE_SENSOR_DOMAIN);
    hasher.update(presence_mask.to_le_bytes());
    for value in values {
        hasher.update(value.unwrap_or(0).to_le_bytes());
    }
    Digest::from_bytes(hasher.finalize().into())
}

/// Derives the value one lane carries after the authority scale.
pub(super) fn scaled_authority(lane_bits: u32, basis_points: u16) -> u32 {
    scaled(lane_bits, basis_points)
}

/// Derives the hover force one declared scale produces from one baseline.
pub(super) fn scaled_hover_force(baseline_bits: u32, basis_points: u16) -> u32 {
    scaled(baseline_bits, basis_points)
}

fn scaled(bits: u32, basis_points: u16) -> u32 {
    let value = f32::from_bits(bits);
    let result =
        (f64::from(value) * f64::from(basis_points) / f64::from(NOMINAL_BASIS_POINTS)) as f32;
    result.to_bits()
}

fn interval_hasher(
    condition_digest: Digest,
    run_seed: u64,
    interval_epoch: u64,
    interval_index: u64,
    first_sequence: u64,
) -> Sha256 {
    let mut hasher = Sha256::new();
    hasher.update(COMMAND_HOLD_DOMAIN);
    hasher.update(condition_digest.as_bytes());
    hasher.update(run_seed.to_le_bytes());
    hasher.update(interval_epoch.to_le_bytes());
    hasher.update(interval_index.to_le_bytes());
    hasher.update(first_sequence.to_le_bytes());
    hasher
}

fn permutation_value(
    condition_digest: Digest,
    run_seed: u64,
    interval_epoch: u64,
    interval_index: u64,
    first_sequence: u64,
    cursor: u64,
) -> u64 {
    let mut hasher = interval_hasher(
        condition_digest,
        run_seed,
        interval_epoch,
        interval_index,
        first_sequence,
    );
    hasher.update(cursor.to_le_bytes());
    let bytes = hasher.finalize();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

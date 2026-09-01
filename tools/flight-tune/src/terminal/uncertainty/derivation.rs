//! The seeded decisions one declaration states, derived again.
//!
//! Every decision under uncertainty is a pure function of the condition
//! identity, the run seed, and a position in the run. Nothing here reads a
//! clock, a counter, or a reported result, so a reader holding only the
//! declaration can state what the executor was required to do and compare
//! it with what the executor says it did.
//!
//! The derivation is a cross-repository contract. It is stated here in the
//! primitive terms one declaration carries; the contract crate states the
//! same values from the artifact request, and a test requires the two to
//! agree on a golden condition.

use sha2::{Digest as ShaDigest, Sha256};

use super::super::invalid_terminal;
use super::{DeclaredCommandHold, EXECUTED_SENSOR_LANE_COUNT, NOMINAL_BASIS_POINTS};
use crate::{Digest, TuneError};

const SENSOR_NOISE_DOMAIN: &[u8] = b"pilotage-sensor-noise-v1";
const COMMAND_HOLD_DOMAIN: &[u8] = b"pilotage-command-hold-v1";
const EFFECTIVE_SENSOR_DOMAIN: &[u8] = b"aviate-effective-sensor-v1";
const SAMPLE_STREAM_DOMAIN: &[u8] = b"pilotage.flight-tune.executed-uncertainty-sample.v1\0";

/// Derives the held offset for one lane and one update bucket.
///
/// The preimage is the noise domain, the condition identity, the run seed,
/// the one-byte lane tag, and the bucket. The first four digest bytes are an
/// unsigned sample that maps onto the closed interval from minus one through
/// one and then scales by the peak amplitude. Neither the amplitude nor the
/// interval enters the digest, so only the lane tag separates two lanes.
#[must_use]
pub fn sensor_offset(
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
#[must_use]
pub fn interval_identity(
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
/// The positions come from a descending shuffle whose swap targets are
/// digest words of the interval identity preimage. The shuffle takes the
/// remainder without rejection, so the same slight bias must appear in
/// every implementation of this contract.
///
/// # Errors
///
/// Returns [`TuneError`] when the declared interval cannot address its own
/// positions on this platform.
pub fn hold_schedule(
    condition_digest: Digest,
    run_seed: u64,
    interval_epoch: u64,
    interval_index: u64,
    first_sequence: u64,
    hold: DeclaredCommandHold,
) -> Result<Vec<bool>, TuneError> {
    let size = usize::try_from(hold.decision_interval_samples)
        .map_err(|_| invalid_terminal("a declared decision interval is not addressable"))?;
    let mut positions = (0..size).collect::<Vec<_>>();
    for cursor in (1..positions.len()).rev() {
        let encoded = u64::try_from(cursor)
            .map_err(|_| invalid_terminal("a decision interval position is not addressable"))?;
        let value = permutation_value(
            condition_digest,
            run_seed,
            interval_epoch,
            interval_index,
            first_sequence,
            encoded,
        );
        let swap = usize::try_from(value % encoded.wrapping_add(1))
            .map_err(|_| invalid_terminal("a decision interval swap is not addressable"))?;
        positions.swap(cursor, swap);
    }
    let count = u64::from(hold.fraction_basis_points) * u64::from(hold.decision_interval_samples)
        / u64::from(NOMINAL_BASIS_POINTS);
    let count = usize::try_from(count)
        .map_err(|_| invalid_terminal("a declared hold count is not addressable"))?;
    let mut decisions = vec![false; size];
    for position in positions.into_iter().take(count) {
        decisions[position] = true;
    }
    Ok(decisions)
}

/// Derives the identity of one exact sensor sample.
///
/// The preimage is the effective-sensor domain, the presence mask, and the
/// twelve lane values in stable order. An absent lane contributes a zero
/// word, so a sample that drops a lane cannot carry the identity of one
/// that kept it.
#[must_use]
pub fn sensor_sample_digest(
    presence_mask: u16,
    values: &[Option<u32>; EXECUTED_SENSOR_LANE_COUNT],
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
///
/// The scale is a ratio of basis points taken in the wider domain with one
/// narrowing at the end, so an implementation that scales in the narrow
/// domain produces a different last bit and is refused.
#[must_use]
pub fn scaled_authority(lane_bits: u32, basis_points: u16) -> u32 {
    let lane = f32::from_bits(lane_bits);
    let scaled =
        (f64::from(lane) * f64::from(basis_points) / f64::from(NOMINAL_BASIS_POINTS)) as f32;
    scaled.to_bits()
}

/// Derives the hover force one declared scale produces from one baseline.
#[must_use]
pub fn scaled_hover_force(baseline_bits: u32, basis_points: u16) -> u32 {
    let baseline = f32::from_bits(baseline_bits);
    let effective =
        (f64::from(baseline) * f64::from(basis_points) / f64::from(NOMINAL_BASIS_POINTS)) as f32;
    effective.to_bits()
}

/// Extends one sample-stream identity by the next sample.
///
/// The chain covers every sample in order, so a stream that drops, adds, or
/// reorders one sample carries another identity than the one its receipt
/// states.
///
/// # Errors
///
/// Returns [`TuneError`] when the sample cannot be encoded.
pub fn extend_sample_stream(
    previous: Digest,
    sample: &super::ExecutedSample,
) -> Result<Digest, TuneError> {
    let bytes = serde_json::to_vec(sample).map_err(|source| TuneError::Encode {
        document: "executed uncertainty sample",
        source,
    })?;
    let mut hasher = Sha256::new();
    hasher.update(SAMPLE_STREAM_DOMAIN);
    hasher.update(previous.as_bytes());
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(&bytes);
    Ok(Digest::from_bytes(hasher.finalize().into()))
}

/// Returns the identity of an empty sample stream.
#[must_use]
pub fn empty_sample_stream() -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(SAMPLE_STREAM_DOMAIN);
    Digest::from_bytes(hasher.finalize().into())
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

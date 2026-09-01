//! The deterministic reference value for one sensor noise lane.
//!
//! The reference is the value a simulator must apply and a verifier must
//! recompute. It uses simulation sample identity only. It never uses wall
//! time, so a repeated run gives the same lane offsets.
//!
//! The derivation is a cross-repository contract. The executor that flies
//! this contract derives the same offset from the same inputs, so the two
//! implementations must stay identical byte for byte.

use sha2::{Digest as ShaDigest, Sha256};

use super::{SensorAxis, SensorNoiseLane};
use crate::Digest;

const SENSOR_NOISE_DOMAIN: &[u8] = b"pilotage-sensor-noise-v1";

/// One physical sensor lane in flight-controller input order.
///
/// The discriminant is the one-byte lane tag in the noise preimage, so the
/// order of these variants is part of the derivation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SensorReferenceLane {
    /// Accelerometer X in meters per second squared.
    AccelerometerX = 0,
    /// Accelerometer Y in meters per second squared.
    AccelerometerY = 1,
    /// Accelerometer Z in meters per second squared.
    AccelerometerZ = 2,
    /// Gyroscope X in radians per second.
    GyroscopeX = 3,
    /// Gyroscope Y in radians per second.
    GyroscopeY = 4,
    /// Gyroscope Z in radians per second.
    GyroscopeZ = 5,
    /// Magnetometer X in microteslas.
    MagnetometerX = 6,
    /// Magnetometer Y in microteslas.
    MagnetometerY = 7,
    /// Magnetometer Z in microteslas.
    MagnetometerZ = 8,
    /// Absolute pressure in pascals.
    AbsolutePressure = 9,
    /// Differential pressure in pascals.
    DifferentialPressure = 10,
    /// Pressure altitude in meters.
    PressureAltitude = 11,
}

impl SensorReferenceLane {
    /// Returns the fixed wire index for this lane.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Returns the fixed presence bit for this lane.
    #[must_use]
    pub const fn presence_bit(self) -> u16 {
        1_u16 << (self as u8)
    }
}

/// The reference perturbation for one lane at one simulation sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorNoiseReference {
    lane: SensorReferenceLane,
    update_bucket: u64,
    offset: f32,
}

impl SensorNoiseReference {
    /// Builds the reference perturbation for one lane and global sample.
    ///
    /// The lane holds its value for a complete update interval, so a
    /// simulator that reads the same sample twice applies the same offset.
    /// The offset is a physical value in the flight-controller lane unit,
    /// which is the SI unit rather than the unit the artifact declares.
    #[must_use]
    pub fn new(
        condition_digest: Digest,
        run_seed: u64,
        global_sample_sequence: u64,
        request: SensorNoiseLane,
    ) -> Self {
        let (lane, peak_amplitude, update_interval_samples) = request_values(request);
        let update_bucket = global_sample_sequence / u64::from(update_interval_samples);
        let offset = bounded_offset(
            condition_digest,
            run_seed,
            lane,
            update_bucket,
            peak_amplitude,
        );
        Self {
            lane,
            update_bucket,
            offset,
        }
    }

    /// Returns the lane that this reference perturbs.
    #[must_use]
    pub const fn lane(self) -> SensorReferenceLane {
        self.lane
    }

    /// Returns the global zero-order-hold bucket for this sample.
    #[must_use]
    pub const fn update_bucket(self) -> u64 {
        self.update_bucket
    }

    /// Returns the exact offset the executor adds to the raw sensor value.
    ///
    /// The magnitude never exceeds the requested peak amplitude.
    #[must_use]
    pub const fn offset(self) -> f32 {
        self.offset
    }
}

/// Resolves one lane request into its derivation inputs.
///
/// The amplitude leaves this function in the flight-controller lane unit. A
/// magnetometer request declares gauss and a pressure request declares
/// hectopascals, so each converts here, before the amplitude reaches the
/// offset. The artifact bound stays in the declared unit.
fn request_values(request: SensorNoiseLane) -> (SensorReferenceLane, f32, u32) {
    match request {
        SensorNoiseLane::Accelerometer {
            axis,
            peak_amplitude_mps2,
            update_interval_samples,
        } => (
            vector_lane(
                axis,
                SensorReferenceLane::AccelerometerX,
                SensorReferenceLane::AccelerometerY,
                SensorReferenceLane::AccelerometerZ,
            ),
            peak_amplitude_mps2 as f32,
            update_interval_samples,
        ),
        SensorNoiseLane::Gyroscope {
            axis,
            peak_amplitude_rad_s,
            update_interval_samples,
        } => (
            vector_lane(
                axis,
                SensorReferenceLane::GyroscopeX,
                SensorReferenceLane::GyroscopeY,
                SensorReferenceLane::GyroscopeZ,
            ),
            peak_amplitude_rad_s as f32,
            update_interval_samples,
        ),
        SensorNoiseLane::Magnetometer {
            axis,
            peak_amplitude_gauss,
            update_interval_samples,
        } => (
            vector_lane(
                axis,
                SensorReferenceLane::MagnetometerX,
                SensorReferenceLane::MagnetometerY,
                SensorReferenceLane::MagnetometerZ,
            ),
            (peak_amplitude_gauss * 100.0) as f32,
            update_interval_samples,
        ),
        SensorNoiseLane::AbsolutePressure {
            peak_amplitude_hpa,
            update_interval_samples,
        } => (
            SensorReferenceLane::AbsolutePressure,
            (peak_amplitude_hpa * 100.0) as f32,
            update_interval_samples,
        ),
        SensorNoiseLane::DifferentialPressure {
            peak_amplitude_hpa,
            update_interval_samples,
        } => (
            SensorReferenceLane::DifferentialPressure,
            (peak_amplitude_hpa * 100.0) as f32,
            update_interval_samples,
        ),
        SensorNoiseLane::PressureAltitude {
            peak_amplitude_m,
            update_interval_samples,
        } => (
            SensorReferenceLane::PressureAltitude,
            peak_amplitude_m as f32,
            update_interval_samples,
        ),
    }
}

fn vector_lane(
    axis: SensorAxis,
    x: SensorReferenceLane,
    y: SensorReferenceLane,
    z: SensorReferenceLane,
) -> SensorReferenceLane {
    match axis {
        SensorAxis::X => x,
        SensorAxis::Y => y,
        SensorAxis::Z => z,
    }
}

/// Derives the bounded offset for one lane and one update bucket.
///
/// The preimage starts with the sensor-noise domain and the condition
/// digest. It then has the run seed, the one-byte lane tag, and the update
/// bucket. Each integer is an unsigned 64-bit little-endian value. The first
/// four preimage-digest bytes are an unsigned 32-bit little-endian sample,
/// which maps onto the closed interval from minus one through one and then
/// scales by the peak amplitude. Neither the amplitude nor the update
/// interval enters the hash, so only the lane tag separates two lanes.
fn bounded_offset(
    condition_digest: Digest,
    run_seed: u64,
    lane: SensorReferenceLane,
    update_bucket: u64,
    peak_amplitude: f32,
) -> f32 {
    let mut hasher = Sha256::new();
    hasher.update(SENSOR_NOISE_DOMAIN);
    hasher.update(condition_digest.as_bytes());
    hasher.update(run_seed.to_le_bytes());
    hasher.update([lane as u8]);
    hasher.update(update_bucket.to_le_bytes());
    let bytes = hasher.finalize();
    let sample = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let unit = f64::from(sample) / f64::from(u32::MAX);
    (unit.mul_add(2.0, -1.0) * f64::from(peak_amplitude)) as f32
}

//! The deterministic reference value for one sensor noise lane.
//!
//! The reference is the value a simulator must apply and a verifier must
//! recompute. It uses simulation sample identity only. It never uses wall
//! time, so a repeated run gives the same lane offsets.

use sha2::{Digest as ShaDigest, Sha256};

use super::{SensorAxis, SensorNoiseLane};
use crate::Digest;

const SENSOR_NOISE_DOMAIN: &[u8] = b"pilotage-sensor-noise-v1";

const ACCELEROMETER_TAG: u8 = 1;
const GYROSCOPE_TAG: u8 = 2;
const MAGNETOMETER_TAG: u8 = 3;
const ABSOLUTE_PRESSURE_TAG: u8 = 4;
const DIFFERENTIAL_PRESSURE_TAG: u8 = 5;
const PRESSURE_ALTITUDE_TAG: u8 = 6;

const SCALAR_AXIS_TAG: u8 = 0;
const AXIS_X_TAG: u8 = 1;
const AXIS_Y_TAG: u8 = 2;
const AXIS_Z_TAG: u8 = 3;

/// One sensor lane identity without its requested amplitude.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SensorReferenceLane {
    /// One accelerometer axis.
    Accelerometer(SensorAxis),
    /// One gyroscope axis.
    Gyroscope(SensorAxis),
    /// One magnetometer axis.
    Magnetometer(SensorAxis),
    /// The absolute-pressure lane.
    AbsolutePressure,
    /// The differential-pressure lane.
    DifferentialPressure,
    /// The pressure-altitude lane.
    PressureAltitude,
}

impl SensorReferenceLane {
    /// Returns the fixed two-byte lane tag.
    ///
    /// The first byte names the sensor. The second byte names the axis, and
    /// is zero for a scalar lane.
    #[must_use]
    pub const fn tag(self) -> [u8; 2] {
        match self {
            Self::Accelerometer(axis) => [ACCELEROMETER_TAG, axis_tag(axis)],
            Self::Gyroscope(axis) => [GYROSCOPE_TAG, axis_tag(axis)],
            Self::Magnetometer(axis) => [MAGNETOMETER_TAG, axis_tag(axis)],
            Self::AbsolutePressure => [ABSOLUTE_PRESSURE_TAG, SCALAR_AXIS_TAG],
            Self::DifferentialPressure => [DIFFERENTIAL_PRESSURE_TAG, SCALAR_AXIS_TAG],
            Self::PressureAltitude => [PRESSURE_ALTITUDE_TAG, SCALAR_AXIS_TAG],
        }
    }
}

/// The reference perturbation for one lane at one simulation sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorNoiseReference {
    lane: SensorReferenceLane,
    update_index: u64,
    offset: f64,
}

impl SensorNoiseReference {
    /// Builds the reference perturbation for one lane and global sample.
    ///
    /// The lane holds its value for a complete update interval, so a
    /// simulator that reads the same sample twice applies the same offset.
    #[must_use]
    pub fn new(
        condition_digest: Digest,
        run_seed: u64,
        global_sample_sequence: u64,
        lane: SensorNoiseLane,
    ) -> Self {
        let interval = u64::from(lane.update_interval_samples()).max(1);
        let update_index = global_sample_sequence / interval;
        let reference_lane = lane.reference_lane();
        let signed = signed_unit_interval(preimage_value(
            condition_digest,
            run_seed,
            reference_lane,
            update_index,
        ));
        Self {
            lane: reference_lane,
            update_index,
            offset: signed * lane.peak_amplitude(),
        }
    }

    /// Returns the lane that this reference perturbs.
    #[must_use]
    pub const fn lane(self) -> SensorReferenceLane {
        self.lane
    }

    /// Returns the zero-order-hold update index for this sample.
    #[must_use]
    pub const fn update_index(self) -> u64 {
        self.update_index
    }

    /// Returns the physical offset to add to the raw sensor value.
    ///
    /// The magnitude never exceeds the requested peak amplitude.
    #[must_use]
    pub const fn offset(self) -> f64 {
        self.offset
    }
}

const fn axis_tag(axis: SensorAxis) -> u8 {
    match axis {
        SensorAxis::X => AXIS_X_TAG,
        SensorAxis::Y => AXIS_Y_TAG,
        SensorAxis::Z => AXIS_Z_TAG,
    }
}

/// Digests the fixed-width sensor preimage.
///
/// The preimage starts with the sensor-noise domain and condition digest. It
/// then has the run seed, the two-byte lane tag, and the update index. Each
/// integer is an unsigned 64-bit little-endian value.
fn preimage_value(
    condition_digest: Digest,
    run_seed: u64,
    lane: SensorReferenceLane,
    update_index: u64,
) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(SENSOR_NOISE_DOMAIN);
    hasher.update(condition_digest.as_bytes());
    hasher.update(run_seed.to_le_bytes());
    hasher.update(lane.tag());
    hasher.update(update_index.to_le_bytes());
    let bytes = hasher.finalize();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Maps a digest word onto the closed interval from minus one through one.
fn signed_unit_interval(value: u64) -> f64 {
    let unit = (value >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64));
    unit.mul_add(2.0, -1.0)
}

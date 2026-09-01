//! Deterministic sensor perturbation contracts.
//!
//! A sensor request acts on the flight-controller sensor input. It must not
//! change the simulator truth stream that supplies score evidence.
//!
//! A request declares its amplitude in the unit of its own field name, and
//! the bound below holds that same declared unit. The conversion to the
//! flight-controller lane unit belongs to the derivation, not to validation.

use serde::{Deserialize, Serialize};

mod reference;

pub use reference::{SensorNoiseReference, SensorReferenceLane};

use crate::{
    ValidationError,
    validation::{finite, nonempty_count},
};

#[cfg(test)]
mod tests;

const MAX_SENSOR_NOISE_LANES: usize = 12;
const MAX_UPDATE_INTERVAL_SAMPLES: u32 = 100_000;
const MAX_ACCELEROMETER_AMPLITUDE_MPS2: f64 = 20.0;
const MAX_GYROSCOPE_AMPLITUDE_RAD_S: f64 = 10.0;
const MAX_MAGNETOMETER_AMPLITUDE_GAUSS: f64 = 2.0;
const MAX_PRESSURE_AMPLITUDE_HPA: f64 = 200.0;
const MAX_PRESSURE_ALTITUDE_AMPLITUDE_M: f64 = 2_000.0;

/// One axis of a vector sensor.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorAxis {
    /// The X axis.
    X,
    /// The Y axis.
    Y,
    /// The Z axis.
    Z,
}

/// One sensor lane and its bounded physical noise request.
///
/// A vector lane names its axis. A scalar lane rejects an axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "sensor", rename_all = "snake_case", deny_unknown_fields)]
pub enum SensorNoiseLane {
    /// Accelerometer noise in meters per second squared.
    Accelerometer {
        /// The vector axis.
        axis: SensorAxis,
        /// The physical peak amplitude.
        peak_amplitude_mps2: f64,
        /// The number of samples between deterministic updates.
        update_interval_samples: u32,
    },
    /// Gyroscope noise in radians per second.
    Gyroscope {
        /// The vector axis.
        axis: SensorAxis,
        /// The physical peak amplitude.
        peak_amplitude_rad_s: f64,
        /// The number of samples between deterministic updates.
        update_interval_samples: u32,
    },
    /// Magnetometer noise in gauss.
    Magnetometer {
        /// The vector axis.
        axis: SensorAxis,
        /// The physical peak amplitude.
        peak_amplitude_gauss: f64,
        /// The number of samples between deterministic updates.
        update_interval_samples: u32,
    },
    /// Absolute-pressure noise in hectopascals.
    AbsolutePressure {
        /// The physical peak amplitude.
        peak_amplitude_hpa: f64,
        /// The number of samples between deterministic updates.
        update_interval_samples: u32,
    },
    /// Differential-pressure noise in hectopascals.
    DifferentialPressure {
        /// The physical peak amplitude.
        peak_amplitude_hpa: f64,
        /// The number of samples between deterministic updates.
        update_interval_samples: u32,
    },
    /// Pressure-altitude noise in meters.
    PressureAltitude {
        /// The physical peak amplitude.
        peak_amplitude_m: f64,
        /// The number of samples between deterministic updates.
        update_interval_samples: u32,
    },
}

/// Sensor perturbations for one condition set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SensorCondition {
    /// Do not change a sensor lane.
    None {},
    /// Apply seeded bounded noise to the listed lanes.
    BoundedNoise {
        /// Exact sensor-lane requests.
        lanes: Vec<SensorNoiseLane>,
    },
}

impl SensorCondition {
    /// Returns the nominal sensor condition.
    #[must_use]
    pub const fn nominal() -> Self {
        Self::None {}
    }

    /// Reports whether no sensor perturbation is requested.
    #[must_use]
    pub const fn is_nominal(&self) -> bool {
        matches!(self, Self::None {})
    }

    /// Returns the bounded-noise lanes.
    #[must_use]
    pub fn noise_lanes(&self) -> &[SensorNoiseLane] {
        match self {
            Self::None {} => &[],
            Self::BoundedNoise { lanes } => lanes,
        }
    }

    /// Validates the complete sensor perturbation.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty lane list, a duplicate lane, a
    /// non-finite or zero amplitude, an amplitude outside its physical
    /// bound, or an update interval outside its fixed bound.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let Self::BoundedNoise { lanes } = self else {
            return Ok(());
        };
        nonempty_count(
            "condition_set.sensor.lanes",
            lanes.len(),
            MAX_SENSOR_NOISE_LANES,
        )?;
        for (index, lane) in lanes.iter().enumerate() {
            lane.validate(index)?;
            if lanes[..index]
                .iter()
                .any(|prior| prior.reference_lane() == lane.reference_lane())
            {
                return Err(ValidationError::DuplicateItem {
                    field: "condition_set.sensor.lanes".to_owned(),
                    index,
                });
            }
        }
        Ok(())
    }
}

impl SensorNoiseLane {
    /// Returns the stable lane identity without its requested amplitude.
    #[must_use]
    pub const fn reference_lane(self) -> SensorReferenceLane {
        match self {
            Self::Accelerometer { axis, .. } => match axis {
                SensorAxis::X => SensorReferenceLane::AccelerometerX,
                SensorAxis::Y => SensorReferenceLane::AccelerometerY,
                SensorAxis::Z => SensorReferenceLane::AccelerometerZ,
            },
            Self::Gyroscope { axis, .. } => match axis {
                SensorAxis::X => SensorReferenceLane::GyroscopeX,
                SensorAxis::Y => SensorReferenceLane::GyroscopeY,
                SensorAxis::Z => SensorReferenceLane::GyroscopeZ,
            },
            Self::Magnetometer { axis, .. } => match axis {
                SensorAxis::X => SensorReferenceLane::MagnetometerX,
                SensorAxis::Y => SensorReferenceLane::MagnetometerY,
                SensorAxis::Z => SensorReferenceLane::MagnetometerZ,
            },
            Self::AbsolutePressure { .. } => SensorReferenceLane::AbsolutePressure,
            Self::DifferentialPressure { .. } => SensorReferenceLane::DifferentialPressure,
            Self::PressureAltitude { .. } => SensorReferenceLane::PressureAltitude,
        }
    }

    fn validate(self, index: usize) -> Result<(), ValidationError> {
        let (name, amplitude, maximum, interval) = self.bounds();
        let field = format!("condition_set.sensor.lanes[{index}].{name}");
        finite(&field, amplitude)?;
        if amplitude <= 0.0 || amplitude > maximum {
            return Err(ValidationError::OutOfRange {
                field,
                actual: amplitude,
                minimum: 0.0,
                maximum,
            });
        }
        if (1..=MAX_UPDATE_INTERVAL_SAMPLES).contains(&interval) {
            return Ok(());
        }
        Err(ValidationError::OutOfRange {
            field: format!("condition_set.sensor.lanes[{index}].update_interval_samples"),
            actual: f64::from(interval),
            minimum: 1.0,
            maximum: f64::from(MAX_UPDATE_INTERVAL_SAMPLES),
        })
    }

    const fn bounds(self) -> (&'static str, f64, f64, u32) {
        match self {
            Self::Accelerometer {
                peak_amplitude_mps2,
                update_interval_samples,
                ..
            } => (
                "peak_amplitude_mps2",
                peak_amplitude_mps2,
                MAX_ACCELEROMETER_AMPLITUDE_MPS2,
                update_interval_samples,
            ),
            Self::Gyroscope {
                peak_amplitude_rad_s,
                update_interval_samples,
                ..
            } => (
                "peak_amplitude_rad_s",
                peak_amplitude_rad_s,
                MAX_GYROSCOPE_AMPLITUDE_RAD_S,
                update_interval_samples,
            ),
            Self::Magnetometer {
                peak_amplitude_gauss,
                update_interval_samples,
                ..
            } => (
                "peak_amplitude_gauss",
                peak_amplitude_gauss,
                MAX_MAGNETOMETER_AMPLITUDE_GAUSS,
                update_interval_samples,
            ),
            Self::AbsolutePressure {
                peak_amplitude_hpa,
                update_interval_samples,
            }
            | Self::DifferentialPressure {
                peak_amplitude_hpa,
                update_interval_samples,
            } => (
                "peak_amplitude_hpa",
                peak_amplitude_hpa,
                MAX_PRESSURE_AMPLITUDE_HPA,
                update_interval_samples,
            ),
            Self::PressureAltitude {
                peak_amplitude_m,
                update_interval_samples,
            } => (
                "peak_amplitude_m",
                peak_amplitude_m,
                MAX_PRESSURE_ALTITUDE_AMPLITUDE_M,
                update_interval_samples,
            ),
        }
    }
}

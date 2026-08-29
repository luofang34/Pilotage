//! Deterministic wind, gust, and turbulence contracts.

use core::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::{
    MAX_GUST_EVENTS, ValidationError,
    validation::{count, duration, range},
};

const MAX_WIND_SPEED_MPS: f64 = 100.0;
const MAX_TURBULENCE_AMPLITUDE_MPS: f64 = 20.0;

/// A horizontal wind in the aviation direction convention.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HorizontalWind {
    /// Wind speed in meters per second.
    pub speed_mps: f64,
    /// True direction that the wind comes from, clockwise from north.
    pub direction_deg: f64,
}

/// One trapezoidal gust event.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GustEvent {
    /// Event start in simulator nanoseconds from condition activation.
    pub start_ns: u64,
    /// Linear rise interval in simulator nanoseconds. Zero gives a step.
    pub rise_ns: u64,
    /// Full-amplitude interval in simulator nanoseconds.
    pub hold_ns: u64,
    /// Linear fall interval in simulator nanoseconds. Zero gives a step.
    pub fall_ns: u64,
    /// Gust speed in meters per second.
    pub speed_mps: f64,
    /// True direction that the gust comes from.
    pub direction_deg: f64,
}

/// A deterministic turbulence model.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TurbulenceModel {
    /// No turbulence component.
    None,
    /// Seeded, linearly interpolated horizontal noise.
    BandLimitedNoise {
        /// Maximum turbulence-vector magnitude in meters per second.
        amplitude_mps: f64,
        /// Interval between deterministic noise knots in simulator nanoseconds.
        knot_interval_ns: u64,
    },
}

/// Wind inputs for one condition set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindCondition {
    /// Constant base wind.
    pub steady: HorizontalWind,
    /// Ordered gust events.
    pub gusts: Vec<GustEvent>,
    /// Deterministic turbulence model.
    pub turbulence: TurbulenceModel,
}

/// One resolved wind sample for a simulator backend.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedWind {
    /// Resulting horizontal wind speed in meters per second.
    pub speed_mps: f64,
    /// True direction that the resulting wind comes from.
    pub direction_deg: f64,
    /// North component of air motion in meters per second.
    pub north_mps: f64,
    /// East component of air motion in meters per second.
    pub east_mps: f64,
    /// Magnitude of the deterministic turbulence component.
    pub turbulence_mps: f64,
}

impl WindCondition {
    /// Resolves the wind request for one seed and simulator-time offset.
    #[must_use]
    pub fn resolve(&self, seed: u64, elapsed_ns: u64) -> AppliedWind {
        let mut vector = WindVector::from(self.steady);
        for gust in &self.gusts {
            vector.add_scaled(
                WindVector::from(HorizontalWind {
                    speed_mps: gust.speed_mps,
                    direction_deg: gust.direction_deg,
                }),
                gust.scale_at(elapsed_ns),
            );
        }
        let turbulence = self.turbulence.sample(seed, elapsed_ns);
        vector.add_scaled(turbulence, 1.0);
        vector.applied(turbulence.magnitude())
    }

    /// Validates the wind, gust, and turbulence request.
    ///
    /// # Errors
    ///
    /// Returns an error when a speed, direction, gust interval, or combined
    /// maximum speed is outside its fixed bound.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.steady.validate("condition_set.wind.steady")?;
        count(
            "condition_set.wind.gusts",
            self.gusts.len(),
            MAX_GUST_EVENTS,
        )?;
        let mut maximum_speed = self.steady.speed_mps;
        for (index, gust) in self.gusts.iter().enumerate() {
            gust.validate(index)?;
            maximum_speed += gust.speed_mps;
        }
        self.turbulence.validate()?;
        maximum_speed += self.turbulence.amplitude_mps();
        range(
            "condition_set.wind.maximum_possible_speed_mps",
            maximum_speed,
            0.0,
            MAX_WIND_SPEED_MPS,
        )
    }
}

impl HorizontalWind {
    fn validate(&self, field: &str) -> Result<(), ValidationError> {
        range(
            &format!("{field}.speed_mps"),
            self.speed_mps,
            0.0,
            MAX_WIND_SPEED_MPS,
        )?;
        range(
            &format!("{field}.direction_deg"),
            self.direction_deg,
            0.0,
            360.0,
        )
    }
}

impl GustEvent {
    fn validate(&self, index: usize) -> Result<(), ValidationError> {
        let field = format!("condition_set.wind.gusts[{index}]");
        range(
            &format!("{field}.speed_mps"),
            self.speed_mps,
            0.0,
            MAX_WIND_SPEED_MPS,
        )?;
        range(
            &format!("{field}.direction_deg"),
            self.direction_deg,
            0.0,
            360.0,
        )?;
        if self.rise_ns == 0 && self.hold_ns == 0 && self.fall_ns == 0 {
            return duration(&format!("{field}.duration"), 0);
        }
        self.start_ns
            .checked_add(self.rise_ns)
            .and_then(|end| end.checked_add(self.hold_ns))
            .and_then(|end| end.checked_add(self.fall_ns))
            .ok_or_else(|| ValidationError::TimeOverflow {
                field: format!("{field}.end_ns"),
            })?;
        Ok(())
    }

    fn scale_at(self, elapsed_ns: u64) -> f64 {
        let Some(relative) = elapsed_ns.checked_sub(self.start_ns) else {
            return 0.0;
        };
        let rise_end = self.rise_ns;
        let hold_end = rise_end.saturating_add(self.hold_ns);
        let fall_end = hold_end.saturating_add(self.fall_ns);
        if relative < rise_end {
            return relative as f64 / self.rise_ns as f64;
        }
        if relative < hold_end {
            return 1.0;
        }
        if relative < fall_end && self.fall_ns > 0 {
            return (fall_end - relative) as f64 / self.fall_ns as f64;
        }
        0.0
    }
}

impl TurbulenceModel {
    fn validate(self) -> Result<(), ValidationError> {
        match self {
            Self::None => Ok(()),
            Self::BandLimitedNoise {
                amplitude_mps,
                knot_interval_ns,
            } => {
                range(
                    "condition_set.wind.turbulence.amplitude_mps",
                    amplitude_mps,
                    0.0,
                    MAX_TURBULENCE_AMPLITUDE_MPS,
                )?;
                duration(
                    "condition_set.wind.turbulence.knot_interval_ns",
                    knot_interval_ns,
                )
            }
        }
    }

    const fn amplitude_mps(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::BandLimitedNoise { amplitude_mps, .. } => amplitude_mps,
        }
    }

    fn sample(self, seed: u64, elapsed_ns: u64) -> WindVector {
        match self {
            Self::None => WindVector::default(),
            Self::BandLimitedNoise {
                amplitude_mps,
                knot_interval_ns,
            } => {
                if knot_interval_ns == 0 || !amplitude_mps.is_finite() {
                    return WindVector::default();
                }
                let knot = elapsed_ns / knot_interval_ns;
                let fraction = (elapsed_ns % knot_interval_ns) as f64 / knot_interval_ns as f64;
                noise_vector(seed, knot, amplitude_mps).interpolate(
                    noise_vector(seed, knot.wrapping_add(1), amplitude_mps),
                    fraction,
                )
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct WindVector {
    north: f64,
    east: f64,
}

impl From<HorizontalWind> for WindVector {
    fn from(wind: HorizontalWind) -> Self {
        let radians = wind.direction_deg.to_radians();
        Self {
            north: -wind.speed_mps * radians.cos(),
            east: -wind.speed_mps * radians.sin(),
        }
    }
}

impl WindVector {
    fn add_scaled(&mut self, other: Self, scale: f64) {
        self.north += other.north * scale;
        self.east += other.east * scale;
    }

    fn interpolate(self, other: Self, fraction: f64) -> Self {
        Self {
            north: self.north + (other.north - self.north) * fraction,
            east: self.east + (other.east - self.east) * fraction,
        }
    }

    fn magnitude(self) -> f64 {
        self.north.hypot(self.east)
    }

    fn applied(self, turbulence_mps: f64) -> AppliedWind {
        let speed_mps = self.magnitude();
        let direction_deg = if speed_mps <= f64::EPSILON {
            0.0
        } else {
            (-self.east)
                .atan2(-self.north)
                .to_degrees()
                .rem_euclid(360.0)
        };
        AppliedWind {
            speed_mps,
            direction_deg,
            north_mps: self.north,
            east_mps: self.east,
            turbulence_mps,
        }
    }
}

fn noise_vector(seed: u64, knot: u64, amplitude: f64) -> WindVector {
    let angle = unit_interval(splitmix64(seed ^ knot.wrapping_mul(0x9e37_79b9_7f4a_7c15))) * TAU;
    let magnitude = unit_interval(splitmix64(
        seed ^ knot.wrapping_mul(0xbf58_476d_1ce4_e5b9) ^ 1,
    )) * amplitude;
    WindVector {
        north: magnitude * angle.cos(),
        east: magnitude * angle.sin(),
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit_interval(value: u64) -> f64 {
    (value >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64))
}

//! The demand envelope of a fixed wing.
//!
//! A fixed wing has no hover and no stick law to shape: its flight
//! controller reads a velocity demand as a course, an airspeed, and a climb
//! rate, and flies them within its own limits. What the operator side needs
//! is the range those demands may take, stated in the airframe's terms, so
//! a demand is bounded where it is made instead of being refused or
//! reshaped by a multirotor law sized for a 3 m/s vehicle.

use serde::{Deserialize, Serialize};

use crate::validation::ValidationError;

/// Standard gravity, m/s².
const GRAVITY_MPS2: f64 = 9.806_65;

/// The demand limits of one fixed-wing airframe.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedWingEnvelope {
    /// The indicated airspeed the airframe cruises at, m/s.
    pub cruise_airspeed_mps: f64,
    /// The lowest indicated airspeed a demand may ask for, m/s.
    pub min_airspeed_mps: f64,
    /// The highest indicated airspeed a demand may ask for, m/s.
    pub max_airspeed_mps: f64,
    /// The greatest climb rate a demand may ask for, m/s.
    pub max_climb_mps: f64,
    /// The greatest sink rate a demand may ask for, m/s.
    pub max_sink_mps: f64,
    /// The bank angle route guidance turns at, radians.
    pub route_bank_rad: f64,
}

impl FixedWingEnvelope {
    /// The Cessna 172 that Simulate flies on Aviate's fixed-wing law.
    pub const C172: Self = Self {
        cruise_airspeed_mps: 55.0,
        min_airspeed_mps: 33.0,
        max_airspeed_mps: 65.0,
        max_climb_mps: 2.5,
        max_sink_mps: 5.0,
        // 25 degrees.
        route_bank_rad: 0.436_332_312_998_582_4,
    };

    /// The rate of turn at the route bank, rad/s, at `speed_mps`: a level
    /// coordinated turn at bank φ turns at g·tan(φ)/v.
    #[must_use]
    pub fn turn_rate_rps(&self, speed_mps: f64) -> f64 {
        GRAVITY_MPS2 * self.route_bank_rad.tan() / speed_mps
    }

    /// The fastest rate of turn the envelope allows: the route bank at the
    /// lowest airspeed.
    #[must_use]
    pub fn max_turn_rate_rps(&self) -> f64 {
        self.turn_rate_rps(self.min_airspeed_mps)
    }
}

/// A fixed-wing envelope whose ranges are ordered and finite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValidatedFixedWingEnvelope(FixedWingEnvelope);

impl ValidatedFixedWingEnvelope {
    /// Validates and wraps an envelope.
    ///
    /// # Errors
    ///
    /// [`ValidationError::FieldOutOfRange`] for a value that is not finite
    /// or not greater than zero, or a bank outside (0°, 60°]; and
    /// [`ValidationError::InvalidOrder`] when the cruise airspeed is not
    /// inside the airspeed range.
    pub fn new(envelope: FixedWingEnvelope) -> Result<Self, ValidationError> {
        let positive = |value: f64, field| {
            if value.is_finite() && value > 0.0 {
                Ok(())
            } else {
                Err(ValidationError::FieldOutOfRange { field })
            }
        };
        positive(envelope.min_airspeed_mps, "fixed_wing.min_airspeed_mps")?;
        positive(
            envelope.cruise_airspeed_mps,
            "fixed_wing.cruise_airspeed_mps",
        )?;
        positive(envelope.max_airspeed_mps, "fixed_wing.max_airspeed_mps")?;
        positive(envelope.max_climb_mps, "fixed_wing.max_climb_mps")?;
        positive(envelope.max_sink_mps, "fixed_wing.max_sink_mps")?;
        positive(envelope.route_bank_rad, "fixed_wing.route_bank_rad")?;
        // Beyond 60 degrees a level turn needs more than 2 g, which no
        // route asks of a fixed wing.
        if envelope.route_bank_rad > core::f64::consts::FRAC_PI_3 {
            return Err(ValidationError::FieldOutOfRange {
                field: "fixed_wing.route_bank_rad",
            });
        }
        if envelope.min_airspeed_mps > envelope.cruise_airspeed_mps {
            return Err(ValidationError::InvalidOrder {
                lower: "fixed_wing.min_airspeed_mps",
                upper: "fixed_wing.cruise_airspeed_mps",
            });
        }
        if envelope.cruise_airspeed_mps > envelope.max_airspeed_mps {
            return Err(ValidationError::InvalidOrder {
                lower: "fixed_wing.cruise_airspeed_mps",
                upper: "fixed_wing.max_airspeed_mps",
            });
        }
        Ok(Self(envelope))
    }

    /// The validated envelope.
    #[must_use]
    pub const fn envelope(&self) -> &FixedWingEnvelope {
        &self.0
    }
}

#[cfg(test)]
mod tests;

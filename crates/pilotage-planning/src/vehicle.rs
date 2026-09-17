//! Reusable vehicle cruise profiles for local planning.

use crate::{PlanningError, error::invalid, navigation::text};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// Reference used by the cruise speed in a vehicle profile.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeedReference {
    /// True airspeed. Route time assumes still air.
    TrueAirspeed,
    /// Speed over the ground for a surface vehicle.
    GroundSpeed,
}

/// A reusable profile. Each route retains its selected profile as a fixed value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleProfile {
    /// Stable profile identity.
    pub id: String,
    /// Local profile revision.
    pub revision: u64,
    /// Profile display name.
    pub name: String,
    /// Registration or other vehicle identifier, when supplied.
    pub registration: Option<String>,
    /// Vehicle class: `aircraft`, `rotorcraft`, `drone`, `ground`, or `marine`.
    pub kind: String,
    /// Entered cruise speed in knots.
    pub cruise_speed_knots: f64,
    /// Reference for the entered cruise speed.
    pub speed_reference: SpeedReference,
    /// Manual or other source used to enter performance values.
    pub source: Option<String>,
}

/// A portable JSON document for vehicle profile import and export.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleProfileDocument {
    /// This implementation accepts version 1.
    pub schema_version: u32,
    /// Complete reusable profiles.
    pub profiles: Vec<VehicleProfile>,
}

impl VehicleProfile {
    /// Checks identity, class, and cruise speed before a profile is used.
    pub fn validate(&self) -> Result<(), PlanningError> {
        text("vehicle.id", &self.id)?;
        text("vehicle.name", &self.name)?;
        for (field, value) in [
            ("registration", &self.registration),
            ("source", &self.source),
        ] {
            if let Some(value) = value {
                text(field, value)?;
            }
        }
        if !matches!(
            self.kind.as_str(),
            "aircraft" | "rotorcraft" | "drone" | "ground" | "marine"
        ) {
            return Err(invalid("vehicle.kind", "unsupported vehicle class"));
        }
        if !self.cruise_speed_knots.is_finite()
            || self.cruise_speed_knots <= 0.0
            || self.cruise_speed_knots > 3000.0
        {
            return Err(invalid(
                "cruise_speed_knots",
                "expected a speed above zero and at most 3000 knots",
            ));
        }
        Ok(())
    }
}

impl VehicleProfileDocument {
    /// Checks the document and rejects duplicate profile identities.
    pub fn validate(&self) -> Result<(), PlanningError> {
        if self.schema_version != 1 || self.profiles.len() > 256 {
            return Err(invalid(
                "profiles",
                "expected schema 1 and at most 256 profiles",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for profile in &self.profiles {
            profile.validate()?;
            if !ids.insert(&profile.id) {
                return Err(invalid(&profile.id, "duplicate vehicle profile"));
            }
        }
        Ok(())
    }
}

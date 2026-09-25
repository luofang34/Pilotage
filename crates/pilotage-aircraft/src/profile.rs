//! The versioned aircraft profile and one flight's loading.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AircraftError;

/// SHA-256 of a profile's canonical JSON, in lowercase hexadecimal.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub String);

/// A loading station, such as a seat row or a baggage area.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Station {
    /// Station name, unique in the profile.
    pub name: String,
    /// Arm from the datum, in metres.
    pub arm_m: f64,
    /// Maximum load, in kilograms.
    pub max_kg: f64,
}

/// A fuel tank.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tank {
    /// Tank name, unique in the profile.
    pub name: String,
    /// Arm from the datum, in metres.
    pub arm_m: f64,
    /// Usable fuel, in litres.
    pub usable_l: f64,
}

/// Where the aircraft stores its energy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnergyStore {
    /// Fuel in tanks. Fuel mass changes the weight and balance.
    Fuel {
        /// Fuel density, in kilograms per litre.
        density_kg_per_l: f64,
        /// Fuel tanks.
        tanks: Vec<Tank>,
    },
    /// A battery. Its mass is part of the empty weight and does not change in
    /// flight.
    Battery {
        /// Usable energy when full, in watt-hours.
        usable_wh: f64,
    },
}

/// Rate at which the aircraft uses its stored energy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Draw {
    /// Fuel flow, in litres per hour.
    FuelFlowLph(f64),
    /// Electrical power, in watts.
    PowerW(f64),
}

impl Draw {
    pub(crate) fn per_hour(self) -> f64 {
        match self {
            Self::FuelFlowLph(v) | Self::PowerW(v) => v,
        }
    }

    pub(crate) fn matches(self, store: &EnergyStore) -> bool {
        matches!(
            (self, store),
            (Self::FuelFlowLph(_), EnergyStore::Fuel { .. })
                | (Self::PowerW(_), EnergyStore::Battery { .. })
        )
    }
}

/// Cruise performance at one planned power setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CruiseModel {
    /// True airspeed, in metres per second.
    pub true_airspeed_mps: f64,
    /// Energy draw, in the unit of the profile's energy store.
    pub draw: Draw,
}

/// Configuration, limits, and performance of one aircraft (ADR-0046).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AircraftProfile {
    /// Profile schema version.
    pub schema_version: u32,
    /// ICAO type designator.
    pub type_designator: String,
    /// Registration, when the profile is for one airframe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration: Option<String>,
    /// Source of the data, for example a flight manual revision.
    pub source: String,
    /// Basic empty weight, in kilograms.
    pub empty_weight_kg: f64,
    /// Basic empty weight arm, in metres.
    pub empty_arm_m: f64,
    /// Maximum takeoff weight, in kilograms.
    pub max_takeoff_weight_kg: f64,
    /// Fuel or battery.
    pub energy: EnergyStore,
    /// Loading stations.
    pub stations: Vec<Station>,
    /// Center-of-gravity envelope as (weight kg, arm m) vertices, in order.
    pub cg_envelope: Vec<[f64; 2]>,
    /// Planned cruise performance.
    pub cruise: CruiseModel,
}

impl AircraftProfile {
    /// The content hash that loadings and results name.
    ///
    /// # Errors
    /// Returns an encoding failure.
    pub fn id(&self) -> Result<ProfileId, AircraftError> {
        let bytes = serde_json::to_vec(self).map_err(AircraftError::Encoding)?;
        Ok(ProfileId(format!("{:x}", Sha256::digest(bytes))))
    }

    /// Check that every value is finite and positive where it must be, and
    /// that names are unique.
    ///
    /// # Errors
    /// Names the first refused field.
    pub fn validate(&self) -> Result<(), AircraftError> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let energy = match &self.energy {
            EnergyStore::Fuel {
                density_kg_per_l, ..
            } => positive(*density_kg_per_l),
            EnergyStore::Battery { usable_wh } => positive(*usable_wh),
        };
        let checks: [(&'static str, bool); 7] = [
            ("schema_version", self.schema_version == 1),
            ("empty_weight_kg", positive(self.empty_weight_kg)),
            ("empty_arm_m", self.empty_arm_m.is_finite()),
            (
                "max_takeoff_weight_kg",
                positive(self.max_takeoff_weight_kg),
            ),
            ("energy", energy),
            (
                "cg_envelope",
                self.cg_envelope.len() >= 3
                    && self.cg_envelope.iter().flatten().all(|v| v.is_finite()),
            ),
            (
                "cruise",
                positive(self.cruise.true_airspeed_mps)
                    && positive(self.cruise.draw.per_hour())
                    && self.cruise.draw.matches(&self.energy),
            ),
        ];
        if let Some((field, _)) = checks.iter().find(|(_, ok)| !ok) {
            return Err(AircraftError::InvalidProfile { field });
        }
        let mut names = std::collections::BTreeSet::new();
        let stations = self
            .stations
            .iter()
            .all(|s| s.arm_m.is_finite() && positive(s.max_kg) && names.insert(s.name.as_str()));
        let tanks = self
            .tanks()
            .iter()
            .all(|t| t.arm_m.is_finite() && positive(t.usable_l) && names.insert(t.name.as_str()));
        if !stations || !tanks {
            return Err(AircraftError::InvalidProfile {
                field: "stations and tanks",
            });
        }
        Ok(())
    }

    /// Fuel tanks. A battery aircraft has none.
    #[must_use]
    pub fn tanks(&self) -> &[Tank] {
        match &self.energy {
            EnergyStore::Fuel { tanks, .. } => tanks,
            EnergyStore::Battery { .. } => &[],
        }
    }
}

/// Station weights and fuel for one flight, with revisions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loading {
    /// Profile that this loading is for.
    pub profile: ProfileId,
    /// Revision of this loading record.
    pub revision: u32,
    /// Weight at each named station, in kilograms.
    pub stations_kg: Vec<(String, f64)>,
    /// Fuel in each named tank, in litres. Empty for a battery aircraft.
    pub fuel_l: Vec<(String, f64)>,
    /// Usable battery energy at the start of the flight, in watt-hours. Only
    /// a battery aircraft has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_wh: Option<f64>,
}

#[cfg(test)]
pub(crate) mod tests;

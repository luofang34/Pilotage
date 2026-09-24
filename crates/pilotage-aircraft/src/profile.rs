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

/// Cruise performance at one planned power setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CruiseModel {
    /// True airspeed, in metres per second.
    pub true_airspeed_mps: f64,
    /// Fuel flow, in litres per hour.
    pub fuel_flow_lph: f64,
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
    /// Fuel density, in kilograms per litre.
    pub fuel_density_kg_per_l: f64,
    /// Loading stations.
    pub stations: Vec<Station>,
    /// Fuel tanks.
    pub tanks: Vec<Tank>,
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
        let checks: [(&'static str, bool); 7] = [
            ("schema_version", self.schema_version == 1),
            ("empty_weight_kg", positive(self.empty_weight_kg)),
            ("empty_arm_m", self.empty_arm_m.is_finite()),
            (
                "max_takeoff_weight_kg",
                positive(self.max_takeoff_weight_kg),
            ),
            (
                "fuel_density_kg_per_l",
                positive(self.fuel_density_kg_per_l),
            ),
            (
                "cg_envelope",
                self.cg_envelope.len() >= 3
                    && self.cg_envelope.iter().flatten().all(|v| v.is_finite()),
            ),
            (
                "cruise",
                positive(self.cruise.true_airspeed_mps) && positive(self.cruise.fuel_flow_lph),
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
            .tanks
            .iter()
            .all(|t| t.arm_m.is_finite() && positive(t.usable_l) && names.insert(t.name.as_str()));
        if !stations || !tanks {
            return Err(AircraftError::InvalidProfile {
                field: "stations and tanks",
            });
        }
        Ok(())
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
    /// Fuel in each named tank, in litres.
    pub fuel_l: Vec<(String, f64)>,
}

#[cfg(test)]
pub(crate) mod tests;

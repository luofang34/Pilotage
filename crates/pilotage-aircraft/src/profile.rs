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

    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::FuelFlowLph(_) => "fuel",
            Self::PowerW(_) => "battery",
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

impl EnergyStore {
    /// Name of this store kind, for refusals.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Fuel { .. } => "fuel",
            Self::Battery { .. } => "battery",
        }
    }

    /// Usable capacity: litres of fuel, or watt-hours of battery energy.
    #[must_use]
    pub fn capacity(&self) -> f64 {
        match self {
            Self::Fuel { tanks, .. } => tanks.iter().map(|t| t.usable_l).sum(),
            Self::Battery { usable_wh } => *usable_wh,
        }
    }
}

impl AircraftProfile {
    /// The content hash that loadings and results name.
    ///
    /// The order of stations, tanks, and envelope vertices is part of the
    /// data, so a reordered profile is a new profile. Negative zero hashes as
    /// zero, because the two compare equal.
    ///
    /// # Errors
    /// Refuses a profile that [`Self::validate`] refuses, because
    /// serde_json writes every non-finite value as the same `null`. Returns an
    /// encoding failure.
    pub fn id(&self) -> Result<ProfileId, AircraftError> {
        self.validate()?;
        let mut value = serde_json::to_value(self).map_err(AircraftError::Encoding)?;
        unsign_zeros(&mut value);

        let bytes = serde_json::to_vec(&value).map_err(AircraftError::Encoding)?;
        Ok(ProfileId(format!("{:x}", Sha256::digest(bytes))))
    }

    /// Check that every value is finite and in range, that the envelope is a
    /// simple polygon, and that station and tank names are present and
    /// unique.
    ///
    /// # Errors
    /// Names the first refused field or entry.
    pub fn validate(&self) -> Result<(), AircraftError> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let energy = match &self.energy {
            EnergyStore::Fuel {
                density_kg_per_l, ..
            } => positive(*density_kg_per_l),
            EnergyStore::Battery { usable_wh } => positive(*usable_wh),
        };
        let envelope = self.cg_envelope.iter().flatten().all(|v| v.is_finite())
            && crate::envelope::is_simple(&self.cg_envelope);
        let checks: [(&'static str, bool); 7] = [
            ("schema_version", self.schema_version == 1),
            (
                "empty_weight_kg",
                positive(self.empty_weight_kg)
                    && self.empty_weight_kg <= self.max_takeoff_weight_kg,
            ),
            ("empty_arm_m", self.empty_arm_m.is_finite()),
            (
                "max_takeoff_weight_kg",
                positive(self.max_takeoff_weight_kg),
            ),
            ("energy", energy),
            ("cg_envelope", envelope),
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
        let entries = self
            .stations
            .iter()
            .map(|s| (s.name.as_str(), s.arm_m, s.max_kg))
            .chain(
                self.tanks()
                    .iter()
                    .map(|t| (t.name.as_str(), t.arm_m, t.usable_l)),
            );
        let mut names = std::collections::BTreeSet::new();
        for (name, arm_m, limit) in entries {
            let reason = if name.is_empty() {
                "empty name"
            } else if !names.insert(name) {
                "name listed twice"
            } else if !arm_m.is_finite() {
                "non-finite arm"
            } else if !positive(limit) {
                "limit is not positive"
            } else {
                continue;
            };
            return Err(AircraftError::InvalidProfileEntry {
                name: name.into(),
                reason,
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

fn unsign_zeros(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(n) if n.as_f64() == Some(0.0) => {
            *n = serde_json::Number::from(0);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(unsign_zeros),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(unsign_zeros),
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests;

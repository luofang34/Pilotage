//! One flight's loading and the checks that every calculator shares.

use serde::{Deserialize, Serialize};

use crate::{AircraftError, AircraftProfile, EnergyStore, ProfileId, Remaining, Station, Tank};

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

/// A loading after its checks, with the profile entries that it names.
pub(crate) struct CheckedLoading<'a> {
    pub(crate) profile: ProfileId,
    pub(crate) stations: Vec<(&'a Station, f64)>,
    pub(crate) tanks: Vec<(&'a Tank, f64)>,
    pub(crate) remaining: Remaining,
}

impl Loading {
    /// Usable energy that this loading puts on board: the sum of the tank
    /// fuel, or the start-of-flight battery energy.
    ///
    /// # Errors
    /// Refuses an invalid profile, a loading for another profile, an unknown
    /// or repeated station or tank, a value outside its limit, and energy of
    /// the other store kind.
    pub fn remaining(&self, profile: &AircraftProfile) -> Result<Remaining, AircraftError> {
        Ok(self.check(profile)?.remaining)
    }

    pub(crate) fn check<'a>(
        &self,
        profile: &'a AircraftProfile,
    ) -> Result<CheckedLoading<'a>, AircraftError> {
        let id = profile.id()?;
        if self.profile != id {
            return Err(AircraftError::ProfileMismatch {
                loading: self.profile.0.clone(),
                profile: id.0,
            });
        }
        let offered = match &profile.energy {
            EnergyStore::Fuel { .. } if self.battery_wh.is_some() => Some("battery"),
            EnergyStore::Battery { .. } if !self.fuel_l.is_empty() => Some("fuel"),
            _ => None,
        };
        if let Some(offered) = offered {
            return Err(AircraftError::EnergyKindMismatch {
                store: profile.energy.kind(),
                offered,
            });
        }
        let stations = entries(&self.stations_kg, &profile.stations, |s| {
            (s.name.as_str(), s.max_kg)
        })?;
        let tanks = entries(&self.fuel_l, profile.tanks(), |t| {
            (t.name.as_str(), t.usable_l)
        })?;
        let remaining = match (&profile.energy, self.battery_wh) {
            (EnergyStore::Fuel { .. }, _) => {
                Remaining::FuelL(tanks.iter().map(|(_, litres)| litres).sum())
            }
            (EnergyStore::Battery { usable_wh }, Some(wh)) => {
                if !(wh.is_finite() && wh >= 0.0 && wh <= *usable_wh) {
                    return Err(invalid("battery_wh", "outside the battery capacity"));
                }
                Remaining::BatteryWh(wh)
            }
            (EnergyStore::Battery { .. }, None) => {
                return Err(invalid("battery_wh", "missing for a battery aircraft"));
            }
        };
        Ok(CheckedLoading {
            profile: id,
            stations,
            tanks,
            remaining,
        })
    }
}

/// Match each loading entry to its profile entry, once, inside its limit.
fn entries<'a, T>(
    loaded: &[(String, f64)],
    available: &'a [T],
    name_and_limit: impl Fn(&T) -> (&str, f64),
) -> Result<Vec<(&'a T, f64)>, AircraftError> {
    let mut seen = std::collections::BTreeSet::new();
    loaded
        .iter()
        .map(|(name, value)| {
            if !seen.insert(name.as_str()) {
                return Err(invalid(name, "listed twice"));
            }
            let entry = available
                .iter()
                .find(|e| name_and_limit(e).0 == name)
                .ok_or_else(|| invalid(name, "not in the profile"))?;
            if !(value.is_finite() && *value >= 0.0 && *value <= name_and_limit(entry).1) {
                return Err(invalid(name, "outside its limit"));
            }
            Ok((entry, *value))
        })
        .collect()
}

#[cfg(test)]
mod tests;

fn invalid(name: &str, reason: &'static str) -> AircraftError {
    AircraftError::InvalidLoading {
        name: name.into(),
        reason,
    }
}

//! Weight and balance for one loading.

use crate::{AircraftError, AircraftProfile, EnergyStore, Loading, ProfileId};

/// Total weight and center of gravity, with the inputs that produced them.
#[derive(Clone, Debug, PartialEq)]
pub struct WeightAndBalance {
    /// Profile hash used.
    pub profile: ProfileId,
    /// Loading revision used.
    pub loading_revision: u32,
    /// Total weight, in kilograms.
    pub weight_kg: f64,
    /// Center-of-gravity arm, in metres.
    pub cg_arm_m: f64,
    /// Whether the weight is at or below the maximum takeoff weight.
    pub within_max_weight: bool,
    /// Whether the (weight, arm) point is inside the CG envelope.
    pub within_envelope: bool,
}

/// Compute weight and balance for a loading of a profile.
///
/// # Errors
/// Refuses an invalid profile, a loading for another profile, an unknown
/// station or tank, and a value outside its limit.
pub fn weight_and_balance(
    profile: &AircraftProfile,
    loading: &Loading,
) -> Result<WeightAndBalance, AircraftError> {
    profile.validate()?;
    let id = profile.id()?;
    if loading.profile != id {
        return Err(AircraftError::ProfileMismatch {
            loading: loading.profile.0.clone(),
            profile: id.0,
        });
    }
    loading.remaining(profile)?;
    let items = loaded_items(profile, loading)?;
    let weight = profile.empty_weight_kg + items.iter().map(|(kg, _)| kg).sum::<f64>();
    let moment = profile.empty_weight_kg * profile.empty_arm_m
        + items.iter().map(|(kg, arm)| kg * arm).sum::<f64>();
    let cg_arm_m = moment / weight;
    Ok(WeightAndBalance {
        profile: id,
        loading_revision: loading.revision,
        weight_kg: weight,
        cg_arm_m,
        within_max_weight: weight <= profile.max_takeoff_weight_kg,
        within_envelope: inside(&profile.cg_envelope, [weight, cg_arm_m]),
    })
}

/// Each loaded station and tank as (weight kg, arm m), after its limit check.
fn loaded_items(
    profile: &AircraftProfile,
    loading: &Loading,
) -> Result<Vec<(f64, f64)>, AircraftError> {
    let stations = loading.stations_kg.iter().map(|(name, kg)| {
        let station = profile
            .stations
            .iter()
            .find(|s| &s.name == name)
            .ok_or_else(|| invalid(name, "unknown station"))?;
        if !(kg.is_finite() && *kg >= 0.0 && *kg <= station.max_kg) {
            return Err(invalid(name, "weight outside the station limit"));
        }
        Ok((*kg, station.arm_m))
    });
    let density = match profile.energy {
        EnergyStore::Fuel {
            density_kg_per_l, ..
        } => density_kg_per_l,
        // A battery aircraft has no tanks, so every fuel entry is refused
        // before this density applies.
        EnergyStore::Battery { .. } => 0.0,
    };
    let tanks = loading.fuel_l.iter().map(move |(name, litres)| {
        let tank = profile
            .tanks()
            .iter()
            .find(|t| &t.name == name)
            .ok_or_else(|| invalid(name, "unknown tank"))?;
        if !(litres.is_finite() && *litres >= 0.0 && *litres <= tank.usable_l) {
            return Err(invalid(name, "fuel outside the tank capacity"));
        }
        Ok((litres * density, tank.arm_m))
    });
    stations.chain(tanks).collect()
}

fn invalid(name: &str, reason: &'static str) -> AircraftError {
    AircraftError::InvalidLoading {
        name: name.into(),
        reason,
    }
}

/// Even-odd point-in-polygon test over (weight, arm) vertices.
fn inside(polygon: &[[f64; 2]], [w, a]: [f64; 2]) -> bool {
    let mut inside = false;
    let mut previous = polygon.len() - 1;
    for (current, &[wi, ai]) in polygon.iter().enumerate() {
        let [wj, aj] = polygon[previous];
        if (wi > w) != (wj > w) && a < (aj - ai) * (w - wi) / (wj - wi) + ai {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

#[cfg(test)]
mod tests;

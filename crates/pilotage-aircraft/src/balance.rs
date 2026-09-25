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
    /// Whether the (weight, arm) point is inside the CG envelope or on its
    /// boundary.
    pub within_envelope: bool,
}

/// Compute weight and balance for a loading of a profile.
///
/// # Errors
/// Refuses an invalid profile, a loading for another profile, an unknown or
/// repeated station or tank, a value outside its limit, and energy of the
/// other store kind.
pub fn weight_and_balance(
    profile: &AircraftProfile,
    loading: &Loading,
) -> Result<WeightAndBalance, AircraftError> {
    let checked = loading.check(profile)?;
    let density = match profile.energy {
        EnergyStore::Fuel {
            density_kg_per_l, ..
        } => density_kg_per_l,
        // A battery aircraft has no tanks, so no tank entry reaches here.
        EnergyStore::Battery { .. } => 0.0,
    };
    let items: Vec<(f64, f64)> = checked
        .stations
        .iter()
        .map(|(station, kg)| (*kg, station.arm_m))
        .chain(
            checked
                .tanks
                .iter()
                .map(|(tank, litres)| (litres * density, tank.arm_m)),
        )
        .collect();
    let weight = profile.empty_weight_kg + items.iter().map(|(kg, _)| kg).sum::<f64>();
    let moment = profile.empty_weight_kg * profile.empty_arm_m
        + items.iter().map(|(kg, arm)| kg * arm).sum::<f64>();
    let cg_arm_m = moment / weight;
    Ok(WeightAndBalance {
        profile: checked.profile,
        loading_revision: loading.revision,
        weight_kg: weight,
        cg_arm_m,
        within_max_weight: weight <= profile.max_takeoff_weight_kg,
        within_envelope: crate::envelope::contains(&profile.cg_envelope, [weight, cg_arm_m]),
    })
}

#[cfg(test)]
mod tests;

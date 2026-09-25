//! Endurance and range from the energy on board.

use crate::{AircraftError, AircraftProfile, Draw, EnergyStore, Loading, ProfileId};

/// Usable energy on board, in the unit of its store.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Remaining {
    /// Usable fuel, in litres.
    FuelL(f64),
    /// Usable battery energy, in watt-hours.
    BatteryWh(f64),
}

impl Remaining {
    fn amount(self) -> f64 {
        match self {
            Self::FuelL(v) | Self::BatteryWh(v) => v,
        }
    }

    fn matches(self, store: &EnergyStore) -> bool {
        matches!(
            (self, store),
            (Self::FuelL(_), EnergyStore::Fuel { .. })
                | (Self::BatteryWh(_), EnergyStore::Battery { .. })
        )
    }
}

impl Loading {
    /// Usable energy that this loading puts on board: the sum of the tank
    /// fuel, or the start-of-flight battery energy.
    ///
    /// # Errors
    /// Refuses a loading for another profile, an unknown tank, fuel outside a
    /// tank capacity, a battery value outside the battery capacity, and
    /// energy of the other store kind.
    pub fn remaining(&self, profile: &AircraftProfile) -> Result<Remaining, AircraftError> {
        let id = profile.id()?;
        if self.profile != id {
            return Err(AircraftError::ProfileMismatch {
                loading: self.profile.0.clone(),
                profile: id.0,
            });
        }
        let refuse = |reason| AircraftError::InvalidLoading {
            name: "energy".into(),
            reason,
        };
        match (&profile.energy, self.battery_wh) {
            (EnergyStore::Fuel { tanks, .. }, None) => {
                let litres = self.fuel_l.iter().map(|(name, litres)| {
                    let tank = tanks.iter().find(|t| &t.name == name);
                    match tank {
                        Some(t)
                            if litres.is_finite() && *litres >= 0.0 && *litres <= t.usable_l =>
                        {
                            Ok(*litres)
                        }
                        Some(_) => Err(refuse("fuel outside the tank capacity")),
                        None => Err(refuse("unknown tank")),
                    }
                });
                Ok(Remaining::FuelL(litres.sum::<Result<f64, _>>()?))
            }
            (EnergyStore::Battery { usable_wh }, Some(wh)) if self.fuel_l.is_empty() => {
                if wh.is_finite() && wh >= 0.0 && wh <= *usable_wh {
                    Ok(Remaining::BatteryWh(wh))
                } else {
                    Err(refuse("battery energy outside the battery capacity"))
                }
            }
            _ => Err(refuse("fuel and battery quantities do not mix")),
        }
    }
}

/// Energy on board at one time, from a live sample or a loading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnergyState {
    /// Usable energy on board.
    pub remaining: Remaining,
    /// Measured draw, when a live sample has one.
    pub measured_draw: Option<Draw>,
    /// Time of the sample, in Unix nanoseconds.
    pub at_unix_ns: u64,
}

/// Endurance and still-air range, with the inputs that produced them.
#[derive(Clone, Debug, PartialEq)]
pub struct Endurance {
    /// Profile hash used.
    pub profile: ProfileId,
    /// Energy state used.
    pub energy: EnergyState,
    /// Reserve held back, in minutes.
    pub reserve_minutes: f64,
    /// Draw used: the measured draw, or the profile cruise draw.
    pub draw: Draw,
    /// Endurance before the reserve, in minutes. Zero when the energy is
    /// already inside the reserve.
    pub minutes: f64,
    /// Still-air range at cruise true airspeed, in metres.
    pub still_air_range_m: f64,
    /// Time after which a consumer must recompute, in Unix nanoseconds.
    pub valid_until_unix_ns: u64,
}

/// How long a result stays valid after its energy sample.
const VALIDITY_NS: u64 = 60_000_000_000;

/// Endurance and still-air range from the energy on board.
///
/// A measured draw takes precedence over the profile cruise draw.
///
/// # Errors
/// Refuses an invalid profile, an energy state or draw of the other store
/// kind, and non-finite or negative inputs.
pub fn endurance(
    profile: &AircraftProfile,
    energy: EnergyState,
    reserve_minutes: f64,
) -> Result<Endurance, AircraftError> {
    profile.validate()?;
    let draw = energy.measured_draw.unwrap_or(profile.cruise.draw);
    if !(energy.remaining.matches(&profile.energy) && draw.matches(&profile.energy)) {
        return Err(AircraftError::InvalidLoading {
            name: "energy state".into(),
            reason: "fuel and battery quantities do not mix",
        });
    }
    let amount = energy.remaining.amount();
    let rate = draw.per_hour();
    let valid = amount.is_finite()
        && amount >= 0.0
        && rate.is_finite()
        && rate > 0.0
        && reserve_minutes.is_finite()
        && reserve_minutes >= 0.0;
    if !valid {
        return Err(AircraftError::InvalidLoading {
            name: "energy state".into(),
            reason: "non-finite or negative energy, draw, or reserve",
        });
    }
    let minutes = (amount / rate * 60.0 - reserve_minutes).max(0.0);
    Ok(Endurance {
        profile: profile.id()?,
        energy,
        reserve_minutes,
        draw,
        minutes,
        still_air_range_m: minutes * 60.0 * profile.cruise.true_airspeed_mps,
        valid_until_unix_ns: energy.at_unix_ns.saturating_add(VALIDITY_NS),
    })
}

#[cfg(test)]
mod tests;

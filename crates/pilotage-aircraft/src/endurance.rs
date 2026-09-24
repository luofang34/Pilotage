//! Endurance and range from the fuel on board.

use crate::{AircraftError, AircraftProfile, ProfileId};

/// Fuel on board at one time, from a live sample or a loading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FuelState {
    /// Usable fuel, in litres.
    pub usable_l: f64,
    /// Measured fuel flow, in litres per hour, when a live sample has one.
    pub measured_flow_lph: Option<f64>,
    /// Time of the sample, in Unix nanoseconds.
    pub at_unix_ns: u64,
}

/// Endurance and still-air range, with the inputs that produced them.
#[derive(Clone, Debug, PartialEq)]
pub struct Endurance {
    /// Profile hash used.
    pub profile: ProfileId,
    /// Fuel state used.
    pub fuel: FuelState,
    /// Reserve held back, in minutes.
    pub reserve_minutes: f64,
    /// Fuel flow used: the measured flow, or the profile cruise flow.
    pub flow_lph: f64,
    /// Endurance before the reserve, in minutes. Zero when the fuel is
    /// already inside the reserve.
    pub minutes: f64,
    /// Still-air range at cruise true airspeed, in metres.
    pub still_air_range_m: f64,
    /// Time after which a consumer must recompute, in Unix nanoseconds.
    pub valid_until_unix_ns: u64,
}

/// How long a result stays valid after its fuel sample.
const VALIDITY_NS: u64 = 60_000_000_000;

/// Endurance and still-air range from the fuel on board.
///
/// A measured fuel flow takes precedence over the profile cruise flow.
///
/// # Errors
/// Refuses an invalid profile and non-finite or negative inputs.
pub fn endurance(
    profile: &AircraftProfile,
    fuel: FuelState,
    reserve_minutes: f64,
) -> Result<Endurance, AircraftError> {
    profile.validate()?;
    let flow = fuel
        .measured_flow_lph
        .unwrap_or(profile.cruise.fuel_flow_lph);
    let valid = fuel.usable_l.is_finite()
        && fuel.usable_l >= 0.0
        && flow.is_finite()
        && flow > 0.0
        && reserve_minutes.is_finite()
        && reserve_minutes >= 0.0;
    if !valid {
        return Err(AircraftError::InvalidLoading {
            name: "fuel state".into(),
            reason: "non-finite or negative fuel, flow, or reserve",
        });
    }
    let minutes = (fuel.usable_l / flow * 60.0 - reserve_minutes).max(0.0);
    Ok(Endurance {
        profile: profile.id()?,
        fuel,
        reserve_minutes,
        flow_lph: flow,
        minutes,
        still_air_range_m: minutes * 60.0 * profile.cruise.true_airspeed_mps,
        valid_until_unix_ns: fuel.at_unix_ns.saturating_add(VALIDITY_NS),
    })
}

#[cfg(test)]
mod tests;

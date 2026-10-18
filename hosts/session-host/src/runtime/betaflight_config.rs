//! Fail-closed Betaflight profile and MSP endpoint configuration.

use std::env::VarError;

use pilotage_adapter_betaflight::{BetaflightConfig, BetaflightProfile};

use crate::error::BetaflightConfigError;

const PROFILE: &str = "PILOTAGE_BETAFLIGHT_PROFILE";
const MSP_ENDPOINT: &str = "PILOTAGE_BETAFLIGHT_MSP_ADDR";

pub(crate) fn from_env() -> Result<BetaflightConfig, BetaflightConfigError> {
    config_from_values(std::env::var(PROFILE), std::env::var(MSP_ENDPOINT))
}

fn config_from_values(
    profile: Result<String, VarError>,
    endpoint: Result<String, VarError>,
) -> Result<BetaflightConfig, BetaflightConfigError> {
    let profile = match profile {
        Ok(value) if value == "simulation" => BetaflightProfile::Simulation,
        Ok(value) => return Err(BetaflightConfigError::Profile { value }),
        Err(VarError::NotPresent) => return Err(BetaflightConfigError::ProfileMissing),
        Err(source @ VarError::NotUnicode(_)) => {
            return Err(BetaflightConfigError::Encoding {
                variable: PROFILE,
                source,
            });
        }
    };
    let mut config = BetaflightConfig::new(profile);
    match endpoint {
        Ok(value) => {
            config.msp_endpoint =
                value
                    .parse()
                    .map_err(|source| BetaflightConfigError::Endpoint {
                        variable: MSP_ENDPOINT,
                        value,
                        source,
                    })?;
        }
        Err(VarError::NotPresent) => {}
        Err(source @ VarError::NotUnicode(_)) => {
            return Err(BetaflightConfigError::Encoding {
                variable: MSP_ENDPOINT,
                source,
            });
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests;

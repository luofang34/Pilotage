//! Fail-closed ArduPilot profile and endpoint configuration.

use std::env::VarError;

use pilotage_adapter_ardupilot::{ArduPilotConfig, ArduPilotProfile};

use crate::error::HostError;

const PROFILE: &str = "PILOTAGE_ARDUPILOT_PROFILE";
/// The local UDP endpoint the FC sends to (SITL `--serial0 udpclient:`).
const ENDPOINT: &str = "PILOTAGE_ARDUPILOT_ADDR";
const GIMBAL: &str = "PILOTAGE_ARDUPILOT_GIMBAL";

pub(crate) fn from_env() -> Result<ArduPilotConfig, HostError> {
    config_from_values(
        std::env::var(PROFILE),
        std::env::var(ENDPOINT),
        std::env::var(GIMBAL),
    )
}

fn config_from_values(
    profile: Result<String, VarError>,
    endpoint: Result<String, VarError>,
    gimbal: Result<String, VarError>,
) -> Result<ArduPilotConfig, HostError> {
    let profile = match profile {
        Ok(value) if value == "simulation" => ArduPilotProfile::Simulation,
        Ok(value) => return Err(HostError::ArduPilotProfile { value }),
        Err(VarError::NotPresent) => return Err(HostError::ArduPilotProfileMissing),
        Err(source @ VarError::NotUnicode(_)) => {
            return Err(HostError::ArduPilotEnvEncoding {
                variable: PROFILE,
                source,
            });
        }
    };
    // An absent or unrecognized flag cannot enable a payload control path.
    let enabled = matches!(gimbal.as_deref(), Ok("1") | Ok("true"));
    let mut config = ArduPilotConfig::new(profile).with_gimbal(enabled);
    match endpoint {
        Ok(value) => {
            config.endpoint = value
                .parse()
                .map_err(|source| HostError::ArduPilotEndpoint { value, source })?;
        }
        Err(VarError::NotPresent) => {}
        Err(source @ VarError::NotUnicode(_)) => {
            return Err(HostError::ArduPilotEnvEncoding {
                variable: ENDPOINT,
                source,
            });
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests;

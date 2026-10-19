//! Fail-closed ArduPilot profile and endpoint configuration.

use std::env::VarError;

use pilotage_adapter_ardupilot::{ArduPilotConfig, ArduPilotProfile};

use crate::error::HostError;

const PROFILE: &str = "PILOTAGE_ARDUPILOT_PROFILE";
/// The local UDP endpoint the FC sends to (SITL `--serial0 udpclient:`).
const ENDPOINT: &str = "PILOTAGE_ARDUPILOT_ADDR";

pub(crate) fn from_env() -> Result<ArduPilotConfig, HostError> {
    config_from_values(std::env::var(PROFILE), std::env::var(ENDPOINT))
}

fn config_from_values(
    profile: Result<String, VarError>,
    endpoint: Result<String, VarError>,
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
    let mut config = ArduPilotConfig::new(profile);
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
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::env::VarError;
    use std::net::SocketAddr;

    use super::config_from_values;
    use crate::error::HostError;

    #[test]
    fn the_profile_is_required_and_only_simulation_is_accepted() {
        assert!(matches!(
            config_from_values(Err(VarError::NotPresent), Err(VarError::NotPresent)),
            Err(HostError::ArduPilotProfileMissing)
        ));
        for value in ["physical", "Simulation", ""] {
            assert!(matches!(
                config_from_values(Ok(value.to_owned()), Err(VarError::NotPresent)),
                Err(HostError::ArduPilotProfile { .. })
            ));
        }
    }

    #[test]
    fn the_endpoint_defaults_and_a_bad_one_fails_closed() {
        let config = config_from_values(Ok("simulation".to_owned()), Err(VarError::NotPresent))
            .expect("defaults");
        assert_eq!(config.endpoint, SocketAddr::from(([127, 0, 0, 1], 14_550)));
        let config = config_from_values(
            Ok("simulation".to_owned()),
            Ok("127.0.0.1:14593".to_owned()),
        )
        .expect("explicit endpoint");
        assert_eq!(config.endpoint, SocketAddr::from(([127, 0, 0, 1], 14_593)));
        assert!(matches!(
            config_from_values(Ok("simulation".to_owned()), Ok("no-port".to_owned())),
            Err(HostError::ArduPilotEndpoint { .. })
        ));
    }
}

use std::env::VarError;
use std::net::SocketAddr;

use super::config_from_values;
use crate::error::HostError;

fn absent() -> Result<String, VarError> {
    Err(VarError::NotPresent)
}

#[test]
fn the_profile_is_required_and_only_simulation_is_accepted() {
    assert!(matches!(
        config_from_values(absent(), absent(), absent()),
        Err(HostError::ArduPilotProfileMissing)
    ));
    for value in ["physical", "Simulation", ""] {
        assert!(matches!(
            config_from_values(Ok(value.to_owned()), absent(), absent()),
            Err(HostError::ArduPilotProfile { .. })
        ));
    }
}

#[test]
fn the_endpoint_defaults_and_a_bad_one_fails_closed() {
    let config =
        config_from_values(Ok("simulation".to_owned()), absent(), absent()).expect("defaults");
    assert_eq!(config.endpoint, SocketAddr::from(([127, 0, 0, 1], 14_550)));
    let config = config_from_values(
        Ok("simulation".to_owned()),
        Ok("127.0.0.1:14593".to_owned()),
        absent(),
    )
    .expect("explicit endpoint");
    assert_eq!(config.endpoint, SocketAddr::from(([127, 0, 0, 1], 14_593)));
    assert!(matches!(
        config_from_values(
            Ok("simulation".to_owned()),
            Ok("no-port".to_owned()),
            absent()
        ),
        Err(HostError::ArduPilotEndpoint { .. })
    ));
}

#[test]
fn the_gimbal_path_requires_an_explicit_opt_in() {
    let enabled = |value| {
        config_from_values(Ok("simulation".to_owned()), absent(), value)
            .expect("simulation config")
            .gimbal
    };
    assert!(!enabled(absent()));
    for value in ["1", "true"] {
        assert!(enabled(Ok(value.to_owned())), "{value}");
    }
    for value in ["0", "false", "", "TRUE", "yes", " true", "1 "] {
        assert!(!enabled(Ok(value.to_owned())), "{value}");
    }
    assert!(!enabled(Err(VarError::NotUnicode(
        std::ffi::OsString::from("invalid flag")
    ))));
}

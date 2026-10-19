#![allow(clippy::expect_used, clippy::panic)]

use std::env::VarError;
use std::net::SocketAddr;

use super::config_from_values;
use crate::error::BetaflightConfigError;

fn absent() -> Result<String, VarError> {
    Err(VarError::NotPresent)
}

#[test]
fn a_direct_start_requires_the_simulation_profile_and_defaults_to_sitl_uart1() {
    assert!(matches!(
        config_from_values(absent(), absent()),
        Err(BetaflightConfigError::ProfileMissing)
    ));
    let config =
        config_from_values(Ok("simulation".to_owned()), absent()).expect("simulation profile");
    assert_eq!(
        config.msp_endpoint,
        SocketAddr::from(([127, 0, 0, 1], 5_761))
    );
}

#[test]
fn every_other_profile_is_refused() {
    for value in ["physical", "Simulation", ""] {
        assert!(
            matches!(
                config_from_values(Ok(value.to_owned()), absent()),
                Err(BetaflightConfigError::Profile { value: ref rejected }) if rejected == value
            ),
            "{value:?} must be refused"
        );
    }
}

#[test]
fn a_present_endpoint_is_used_and_a_malformed_one_fails_closed() {
    let config = config_from_values(Ok("simulation".to_owned()), Ok("127.0.0.1:5762".to_owned()))
        .expect("parses");
    assert_eq!(config.msp_endpoint.port(), 5_762);
    assert!(matches!(
        config_from_values(Ok("simulation".to_owned()), Ok("5761".to_owned())),
        Err(BetaflightConfigError::Endpoint { .. })
    ));
}

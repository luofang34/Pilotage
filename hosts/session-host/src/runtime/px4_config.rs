//! Fail-closed PX4 profile and endpoint configuration.

use std::env::VarError;
use std::net::SocketAddr;

use pilotage_adapter_px4::{Px4Config, Px4Profile};

use crate::error::HostError;

const PROFILE: &str = "PILOTAGE_PX4_PROFILE";
const TELEMETRY_ENDPOINT: &str = "PILOTAGE_PX4_ADDR";
const STREAM_COMMAND_ENDPOINT: &str = "PILOTAGE_PX4_GCS_ADDR";
const COMMAND_ENDPOINT: &str = "PILOTAGE_PX4_FC_ADDR";
const GIMBAL: &str = "PILOTAGE_PX4_GIMBAL";
/// Acceptance fault injection: drop the gimbal link-loss stop so the vehicle's
/// own failsafe is the sole mechanism under test. Honoured only under the
/// simulation profile (enforced by [`Px4Config::with_gimbal_stop_dropped`]).
const DROP_GIMBAL_STOP: &str = "PILOTAGE_PX4_DROP_GIMBAL_STOP";

pub(crate) fn from_env() -> Result<Px4Config, HostError> {
    config_from_values(
        std::env::var(PROFILE),
        std::env::var(TELEMETRY_ENDPOINT),
        std::env::var(STREAM_COMMAND_ENDPOINT),
        std::env::var(COMMAND_ENDPOINT),
        std::env::var(GIMBAL),
        std::env::var(DROP_GIMBAL_STOP),
    )
}

fn config_from_values(
    profile: Result<String, VarError>,
    telemetry_endpoint: Result<String, VarError>,
    stream_command_endpoint: Result<String, VarError>,
    command_endpoint: Result<String, VarError>,
    gimbal: Result<String, VarError>,
    drop_gimbal_stop: Result<String, VarError>,
) -> Result<Px4Config, HostError> {
    let profile = parse_profile(profile)?;
    let mut config = Px4Config::new(profile)
        .with_gimbal(parse_flag(gimbal))
        .with_gimbal_stop_dropped(parse_flag(drop_gimbal_stop));
    config.telemetry_endpoint = parse_endpoint(
        TELEMETRY_ENDPOINT,
        telemetry_endpoint,
        config.telemetry_endpoint,
    )?;
    config.stream_command_endpoint = parse_endpoint(
        STREAM_COMMAND_ENDPOINT,
        stream_command_endpoint,
        config.stream_command_endpoint,
    )?;
    config.command_endpoint =
        parse_endpoint(COMMAND_ENDPOINT, command_endpoint, config.command_endpoint)?;
    Ok(config)
}

/// A capability flag: enabled only by an explicit truthy value, so a
/// vehicle without a gimbal never advertises the scope by accident.
fn parse_flag(value: Result<String, VarError>) -> bool {
    matches!(value.as_deref(), Ok("1") | Ok("true"))
}

fn parse_profile(value: Result<String, VarError>) -> Result<Px4Profile, HostError> {
    match value {
        Ok(value) if value == "simulation" => Ok(Px4Profile::Simulation),
        Ok(value) => Err(HostError::Px4Profile { value }),
        Err(VarError::NotPresent) => Err(HostError::Px4ProfileMissing),
        Err(source @ VarError::NotUnicode(_)) => Err(HostError::Px4ProfileEncoding { source }),
    }
}

fn parse_endpoint(
    variable: &'static str,
    value: Result<String, VarError>,
    default: SocketAddr,
) -> Result<SocketAddr, HostError> {
    match value {
        Ok(value) => value.parse().map_err(|source| HostError::Px4Endpoint {
            variable,
            value,
            source,
        }),
        Err(VarError::NotPresent) => Ok(default),
        Err(source @ VarError::NotUnicode(_)) => {
            Err(HostError::Px4EndpointEncoding { variable, source })
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::env::VarError;
    use std::ffi::OsString;
    use std::net::SocketAddr;

    use super::config_from_values;
    use crate::error::HostError;

    fn absent() -> Result<String, VarError> {
        Err(VarError::NotPresent)
    }

    fn config(
        profile: Result<String, VarError>,
    ) -> Result<pilotage_adapter_px4::Px4Config, HostError> {
        config_from_values(profile, absent(), absent(), absent(), absent(), absent())
    }

    #[test]
    fn direct_px4_start_requires_an_explicit_simulation_profile() {
        assert!(matches!(
            config(absent()),
            Err(HostError::Px4ProfileMissing)
        ));
        let accepted = config(Ok("simulation".to_owned())).expect("simulation profile");
        assert_eq!(
            accepted.telemetry_endpoint,
            SocketAddr::from(([127, 0, 0, 1], 14_550))
        );
    }

    #[test]
    fn direct_px4_start_refuses_every_non_simulation_profile() {
        for value in ["physical", "oracle-only", "simulaton", "Simulation", ""] {
            let refusal = config(Ok(value.to_owned()));
            assert!(
                matches!(refusal, Err(HostError::Px4Profile { value: ref rejected }) if rejected == value),
                "{value:?} must be refused, got {refusal:?}"
            );
        }
    }

    #[test]
    fn direct_px4_start_refuses_non_unicode_profile_values() {
        #[cfg(unix)]
        let raw = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![0x66, 0xFF, 0x6F])
        };
        #[cfg(not(unix))]
        let raw = OsString::from("\u{FFFD}garbage");
        let refusal = config(Err(VarError::NotUnicode(raw)));
        assert!(matches!(refusal, Err(HostError::Px4ProfileEncoding { .. })));
    }

    #[test]
    fn present_invalid_endpoints_fail_instead_of_using_defaults() {
        for (variable, refusal) in [
            (
                "PILOTAGE_PX4_ADDR",
                config_from_values(
                    Ok("simulation".to_owned()),
                    Ok("not-an-address".to_owned()),
                    absent(),
                    absent(),
                    absent(),
                    absent(),
                ),
            ),
            (
                "PILOTAGE_PX4_GCS_ADDR",
                config_from_values(
                    Ok("simulation".to_owned()),
                    absent(),
                    Ok("127.0.0.1:not-a-port".to_owned()),
                    absent(),
                    absent(),
                    absent(),
                ),
            ),
            (
                "PILOTAGE_PX4_FC_ADDR",
                config_from_values(
                    Ok("simulation".to_owned()),
                    absent(),
                    absent(),
                    Ok("missing-port".to_owned()),
                    absent(),
                    absent(),
                ),
            ),
        ] {
            assert!(
                matches!(refusal, Err(HostError::Px4Endpoint { variable: rejected, .. }) if rejected == variable),
                "{variable} must fail closed, got {refusal:?}"
            );
        }
    }
}

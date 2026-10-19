//! Fail-closed PX4 profile and endpoint configuration.

use std::env::VarError;
use std::net::SocketAddr;

use pilotage_adapter_px4::{Px4Config, Px4Profile};

use crate::error::HostError;

const PROFILE: &str = "PILOTAGE_PX4_PROFILE";
const TELEMETRY_ENDPOINT: &str = "PILOTAGE_PX4_ADDR";
const STREAM_COMMAND_ENDPOINT: &str = "PILOTAGE_PX4_GCS_ADDR";
const COMMAND_ENDPOINT: &str = "PILOTAGE_PX4_FC_ADDR";
/// The MAVLink system id of the PX4 instance (SITL instance `N` is `N + 1`).
const SYSTEM_ID: &str = "PILOTAGE_PX4_SYSTEM_ID";
const GIMBAL: &str = "PILOTAGE_PX4_GIMBAL";
/// Acceptance fault injection: drop the gimbal link-loss stop so the vehicle's
/// own failsafe is the sole mechanism under test. Honoured only under the
/// simulation profile (enforced by [`Px4Config::with_gimbal_stop_dropped`]).
const DROP_GIMBAL_STOP: &str = "PILOTAGE_PX4_DROP_GIMBAL_STOP";

/// The raw PX4 environment, read once so parsing is testable without
/// mutating the process environment.
#[derive(Debug, Clone)]
struct Px4Env {
    profile: Result<String, VarError>,
    telemetry_endpoint: Result<String, VarError>,
    stream_command_endpoint: Result<String, VarError>,
    command_endpoint: Result<String, VarError>,
    system_id: Result<String, VarError>,
    gimbal: Result<String, VarError>,
    drop_gimbal_stop: Result<String, VarError>,
}

impl Px4Env {
    fn read() -> Self {
        Self {
            profile: std::env::var(PROFILE),
            telemetry_endpoint: std::env::var(TELEMETRY_ENDPOINT),
            stream_command_endpoint: std::env::var(STREAM_COMMAND_ENDPOINT),
            command_endpoint: std::env::var(COMMAND_ENDPOINT),
            system_id: std::env::var(SYSTEM_ID),
            gimbal: std::env::var(GIMBAL),
            drop_gimbal_stop: std::env::var(DROP_GIMBAL_STOP),
        }
    }
}

pub(crate) fn from_env() -> Result<Px4Config, HostError> {
    config_from_env(Px4Env::read())
}

fn config_from_env(env: Px4Env) -> Result<Px4Config, HostError> {
    let profile = parse_profile(env.profile)?;
    let mut config = Px4Config::new(profile)
        .with_gimbal(parse_flag(env.gimbal))
        .with_gimbal_stop_dropped(parse_flag(env.drop_gimbal_stop));
    config.telemetry_endpoint = parse_endpoint(
        TELEMETRY_ENDPOINT,
        env.telemetry_endpoint,
        config.telemetry_endpoint,
    )?;
    config.stream_command_endpoint = parse_endpoint(
        STREAM_COMMAND_ENDPOINT,
        env.stream_command_endpoint,
        config.stream_command_endpoint,
    )?;
    config.command_endpoint = parse_endpoint(
        COMMAND_ENDPOINT,
        env.command_endpoint,
        config.command_endpoint,
    )?;
    config.system_id = parse_system_id(env.system_id, config.system_id)?;
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

/// System id 0 is the MAVLink broadcast address, never a vehicle.
fn parse_system_id(value: Result<String, VarError>, default: u8) -> Result<u8, HostError> {
    match value {
        Ok(value) => match value.parse::<u8>() {
            Ok(id) if id != 0 => Ok(id),
            _ => Err(HostError::Px4Setting {
                variable: SYSTEM_ID,
                value,
                expected: "a MAVLink system id from 1 to 255",
            }),
        },
        Err(VarError::NotPresent) => Ok(default),
        Err(source @ VarError::NotUnicode(_)) => Err(HostError::Px4EndpointEncoding {
            variable: SYSTEM_ID,
            source,
        }),
    }
}

#[cfg(test)]
mod tests;

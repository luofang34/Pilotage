//! Environment configuration of the simulated multicopter adapter.
//!
//! The host reads every variable once, at startup, into one
//! [`SimConfig`]. A malformed value stops the startup.

use std::ffi::OsString;
use std::path::PathBuf;

use pilotage_adapter_sim_multicopter::camera::fault::{FaultParseError, parse_faults};
use pilotage_adapter_sim_multicopter::{
    ESRI_WORLD_IMAGERY, LaunchPoint, SimConfig, TileSourceConfig,
};

const LAUNCH: &str = "PILOTAGE_SIM_LAUNCH";
const NAME: &str = "PILOTAGE_SIM_NAME";
const TILES_URL: &str = "PILOTAGE_SIM_TILES_URL";
const TILE_CACHE: &str = "PILOTAGE_SIM_TILE_CACHE";
const CAMERA_FPS: &str = "PILOTAGE_SIM_CAMERA_FPS";
const CAMERA_FAULT: &str = "PILOTAGE_SIM_CAMERA_FAULT";
const BATTERY_WH: &str = "PILOTAGE_SIM_BATTERY_WH";
const SEED: &str = "PILOTAGE_SIM_SEED";
const GIMBAL_PITCH: &str = "PILOTAGE_SIM_GIMBAL_PITCH";
const FRAME_DUMP: &str = "PILOTAGE_SIM_FRAME_DUMP";
/// The tile template word that selects the offline procedural texture.
const PROCEDURAL: &str = "procedural";
/// The cache directory name under the system temporary directory.
const DEFAULT_CACHE: &str = "pilotage-sim-tiles";

/// A simulated multicopter configuration failure.
#[derive(Debug, thiserror::Error)]
pub enum SimMulticopterHostError {
    /// The launch point is not set.
    #[error("{LAUNCH} must be set to lat_deg,lon_deg,alt_m")]
    MissingLaunch,
    /// A variable is not valid UTF-8.
    #[error("{variable} is not valid UTF-8: {value:?}")]
    Encoding {
        /// The variable.
        variable: &'static str,
        /// The rejected value.
        value: OsString,
    },
    /// A variable has a value outside its accepted form or range.
    #[error("{variable} {value:?} is invalid: {reason}")]
    Invalid {
        /// The variable.
        variable: &'static str,
        /// The rejected value.
        value: String,
        /// The accepted form or range.
        reason: &'static str,
    },
    /// The camera fault list does not parse.
    #[error("{CAMERA_FAULT}: {0}")]
    CameraFault(#[source] FaultParseError),
}

/// Reads the configuration from the process environment.
///
/// # Errors
/// Returns the first missing or malformed variable.
pub(super) fn config_from_env() -> Result<SimConfig, SimMulticopterHostError> {
    config_from(|name| std::env::var_os(name))
}

/// Reads the configuration through `lookup`, which returns a variable's
/// value or `None` when it is not set.
///
/// # Errors
/// Returns the first missing or malformed variable.
pub(super) fn config_from(
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<SimConfig, SimMulticopterHostError> {
    let text = |variable: &'static str| -> Result<Option<String>, SimMulticopterHostError> {
        lookup(variable)
            .map(|value| {
                value
                    .into_string()
                    .map_err(|value| SimMulticopterHostError::Encoding { variable, value })
            })
            .transpose()
    };
    let launch = text(LAUNCH)?.ok_or(SimMulticopterHostError::MissingLaunch)?;
    let mut config = SimConfig::new(parse_launch(&launch)?);
    if let Some(name) = text(NAME)? {
        config.name = parse_name(&name)?;
    }
    let cache_dir = lookup(TILE_CACHE)
        .filter(|value| !value.is_empty())
        .map_or_else(|| std::env::temp_dir().join(DEFAULT_CACHE), PathBuf::from);
    config.camera.tiles = match text(TILES_URL)? {
        None => TileSourceConfig::Http {
            template: ESRI_WORLD_IMAGERY.to_owned(),
            cache_dir: Some(cache_dir),
        },
        Some(word) if word == PROCEDURAL => TileSourceConfig::Procedural,
        Some(template) => TileSourceConfig::Http {
            template: parse_template(&template)?,
            cache_dir: Some(cache_dir),
        },
    };
    if let Some(fps) = text(CAMERA_FPS)? {
        config.camera.fps = fps
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|fps| (1..=30).contains(fps))
            .ok_or_else(|| invalid(CAMERA_FPS, &fps, "1 to 30 frames per second"))?;
    }
    if let Some(faults) = text(CAMERA_FAULT)? {
        config.camera.faults =
            parse_faults(&faults).map_err(SimMulticopterHostError::CameraFault)?;
    }
    if let Some(wh) = text(BATTERY_WH)? {
        config.battery.capacity_wh =
            parse_number(BATTERY_WH, &wh, 0.1, 10_000.0, "0.1 to 10000 watt-hours")?;
    }
    if let Some(seed) = text(SEED)? {
        config.seed = seed
            .parse()
            .map_err(|_| invalid(SEED, &seed, "an unsigned integer"))?;
    }
    if let Some(pitch) = text(GIMBAL_PITCH)? {
        config.initial_gimbal_pitch_deg =
            parse_number(GIMBAL_PITCH, &pitch, -90.0, 60.0, "-90 to 60 degrees")?;
    }
    config.camera.frame_dump = lookup(FRAME_DUMP)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    Ok(config)
}

fn invalid(variable: &'static str, value: &str, reason: &'static str) -> SimMulticopterHostError {
    SimMulticopterHostError::Invalid {
        variable,
        value: value.to_owned(),
        reason,
    }
}

fn parse_launch(value: &str) -> Result<LaunchPoint, SimMulticopterHostError> {
    let fail = || invalid(LAUNCH, value, "expected lat_deg,lon_deg,alt_m");
    let parts = value
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| fail())?;
    let [lat_deg, lon_deg, alt_m] = parts.as_slice() else {
        return Err(fail());
    };
    let in_range = (-85.0..=85.0).contains(lat_deg)
        && (-180.0..=180.0).contains(lon_deg)
        && (-500.0..=9_000.0).contains(alt_m);
    if !in_range {
        return Err(invalid(
            LAUNCH,
            value,
            "latitude -85 to 85, longitude -180 to 180, altitude -500 to 9000 m",
        ));
    }
    Ok(LaunchPoint {
        lat_deg: *lat_deg,
        lon_deg: *lon_deg,
        alt_m: *alt_m,
    })
}

fn parse_name(value: &str) -> Result<String, SimMulticopterHostError> {
    let name = value.trim();
    if name.is_empty() || name.len() > 48 || name.chars().any(char::is_control) {
        return Err(invalid(NAME, value, "1 to 48 printable characters"));
    }
    Ok(name.to_owned())
}

fn parse_template(value: &str) -> Result<String, SimMulticopterHostError> {
    let scheme = value.starts_with("https://") || value.starts_with("http://");
    let fields = ["{z}", "{x}", "{y}"]
        .iter()
        .all(|field| value.contains(field));
    if !scheme || !fields {
        return Err(invalid(
            TILES_URL,
            value,
            "procedural, or an http(s) URL template with {z}, {x}, and {y}",
        ));
    }
    Ok(value.to_owned())
}

fn parse_number(
    variable: &'static str,
    value: &str,
    min: f64,
    max: f64,
    reason: &'static str,
) -> Result<f64, SimMulticopterHostError> {
    let number = value
        .trim()
        .parse::<f64>()
        .map_err(|_| invalid(variable, value, reason))?;
    if !number.is_finite() || !(min..=max).contains(&number) {
        return Err(invalid(variable, value, reason));
    }
    Ok(number)
}

#[cfg(test)]
mod tests;

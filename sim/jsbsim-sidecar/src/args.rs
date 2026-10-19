//! Command-line options. Defaults stand the vehicle on the runway 04
//! threshold of Solberg-Hunterdon Airport (N51) and serve PX4 instance 0.

use std::path::PathBuf;
use std::time::Duration;

use pilotage_world::{DEFAULT_SITE, Site};

use crate::aircraft::{AIRCRAFT_NAMES, Aircraft};

/// Parsed options.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    /// The loopback TCP port PX4 connects to.
    pub port: u16,
    /// The aircraft.
    pub aircraft: Aircraft,
    /// The directory of the aircraft's model, if not its bundled one.
    pub aircraft_dir: Option<PathBuf>,
    /// The JSBSim checkout whose engine and systems data the model uses.
    pub jsbsim_root: PathBuf,
    /// The site: its declared elevation stands where the terrain has no
    /// coverage.
    pub site: Site,
    /// Start latitude, degrees.
    pub latitude_deg: f64,
    /// Start longitude, degrees.
    pub longitude_deg: f64,
    /// Start heading, degrees true.
    pub heading_deg: f64,
    /// A Terrarium MBTiles archive for the ground; a flat world at the
    /// site's elevation without one.
    pub terrain: Option<PathBuf>,
    /// The magnetic epoch, Unix seconds; the start time when absent.
    pub epoch_unix_s: Option<i64>,
    /// The sensor noise seed.
    pub seed: u64,
    /// Simulation seconds per wall second; 0 runs unpaced.
    pub realtime_factor: f64,
    /// How long to wait for PX4's answer to a step.
    pub lockstep_timeout: Duration,
}

/// A malformed command line.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArgsError {
    /// A flag lacks its value.
    #[error("{flag} requires a value")]
    MissingValue {
        /// The flag.
        flag: String,
    },
    /// A value does not parse.
    #[error("{flag} value {value:?} is not valid: {reason}")]
    Invalid {
        /// The flag.
        flag: String,
        /// The value.
        value: String,
        /// What was expected.
        reason: &'static str,
    },
    /// An unknown argument.
    #[error("unrecognized argument {0:?}")]
    Unrecognized(String),
}

impl Default for Args {
    fn default() -> Self {
        Self {
            port: 4_560,
            aircraft: Aircraft::X500,
            aircraft_dir: None,
            jsbsim_root: PathBuf::from(env!("PILOTAGE_JSBSIM_ROOT")),
            site: DEFAULT_SITE,
            latitude_deg: DEFAULT_SITE.latitude_deg,
            longitude_deg: DEFAULT_SITE.longitude_deg,
            heading_deg: DEFAULT_SITE.heading_deg,
            terrain: None,
            epoch_unix_s: None,
            seed: 1,
            realtime_factor: 1.0,
            lockstep_timeout: Duration::from_secs(2),
        }
    }
}

fn parse<T: std::str::FromStr>(
    flag: &str,
    value: String,
    reason: &'static str,
) -> Result<T, ArgsError> {
    value.parse().map_err(|_| ArgsError::Invalid {
        flag: flag.to_owned(),
        value,
        reason,
    })
}

fn finite(flag: &str, value: String) -> Result<f64, ArgsError> {
    let number: f64 = parse(flag, value.clone(), "a number")?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(ArgsError::Invalid {
            flag: flag.to_owned(),
            value,
            reason: "a finite number",
        })
    }
}

impl Args {
    /// Parses `argv` (without the program name).
    ///
    /// # Errors
    ///
    /// Returns [`ArgsError`] for an unknown flag or a malformed value.
    pub fn parse(argv: &[String]) -> Result<Self, ArgsError> {
        let mut args = Self::default();
        let mut iter = argv.iter();
        while let Some(flag) = iter.next() {
            let mut value = || {
                iter.next()
                    .cloned()
                    .ok_or(ArgsError::MissingValue { flag: flag.clone() })
            };
            match flag.as_str() {
                "--port" => args.port = parse(flag, value()?, "a port")?,
                "--aircraft" => {
                    let name = value()?;
                    args.aircraft = Aircraft::parse(&name).ok_or(ArgsError::Invalid {
                        flag: flag.clone(),
                        value: name,
                        reason: "x500 or rascal",
                    })?;
                }
                "--aircraft-dir" => args.aircraft_dir = Some(PathBuf::from(value()?)),
                "--jsbsim-root" => args.jsbsim_root = PathBuf::from(value()?),
                "--site-elevation-m" => args.site.elevation_m = finite(flag, value()?)?,
                "--latitude-deg" => args.latitude_deg = finite(flag, value()?)?,
                "--longitude-deg" => args.longitude_deg = finite(flag, value()?)?,
                "--heading-deg" => args.heading_deg = finite(flag, value()?)?,
                "--terrain" => args.terrain = Some(PathBuf::from(value()?)),
                "--epoch-unix-s" => {
                    args.epoch_unix_s = Some(parse(flag, value()?, "whole seconds")?)
                }
                "--seed" => args.seed = parse(flag, value()?, "an unsigned integer")?,
                "--realtime-factor" => args.realtime_factor = finite(flag, value()?)?,
                "--lockstep-timeout-s" => {
                    args.lockstep_timeout =
                        Duration::from_secs_f64(finite(flag, value()?)?.clamp(0.001, 3_600.0));
                }
                other => return Err(ArgsError::Unrecognized(other.to_owned())),
            }
        }
        if !(-90.0..=90.0).contains(&args.latitude_deg) {
            return Err(ArgsError::Invalid {
                flag: "--latitude-deg".to_owned(),
                value: args.latitude_deg.to_string(),
                reason: "a latitude in [-90, 90]",
            });
        }
        Ok(args)
    }
}

/// The usage text.
pub fn usage() -> String {
    format!(
        "pilotage-jsbsim-sidecar [--port 4560] [--aircraft {}] [--aircraft-dir DIR] \
         [--jsbsim-root DIR] [--latitude-deg D --longitude-deg D --heading-deg D] \
         [--site-elevation-m M] [--terrain ARCHIVE.mbtiles] [--epoch-unix-s S] [--seed N] \
         [--realtime-factor F] [--lockstep-timeout-s S]",
        AIRCRAFT_NAMES.join("|")
    )
}

#[cfg(test)]
mod tests;

//! Command-line options. Defaults stand the vehicle on the runway 04
//! threshold of Solberg-Hunterdon Airport (N51) and serve PX4 instance 0.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use pilotage_world::{DEFAULT_SITE, Site};

use crate::aircraft::{AIRCRAFT_NAMES, Aircraft};
use crate::gimbal::GimbalLimits;
use crate::simulation::STEP_MS_CHOICES;

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
    /// The gimbal PX4 drives, from its `MNT_*` travel parameters.
    pub gimbal: Option<GimbalLimits>,
    /// The state step, ms: one of 1, 2, 4.
    pub step_ms: u32,
    /// Where to send the truth every step, besides PX4: the session
    /// host's simulator truth lane.
    pub truth_addr: Option<SocketAddr>,
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
            gimbal: None,
            truth_addr: None,
            step_ms: 4,
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

/// `MNT_RANGE_ROLL,MNT_MAX_PITCH,MNT_MIN_PITCH,MNT_RANGE_YAW`, degrees.
fn gimbal_limits(flag: &str, value: String) -> Result<GimbalLimits, ArgsError> {
    let invalid = || ArgsError::Invalid {
        flag: flag.to_owned(),
        value: value.clone(),
        reason: "range_roll,max_pitch,min_pitch,range_yaw in degrees",
    };
    let numbers: Vec<f64> = value
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| invalid())?;
    match numbers[..] {
        [roll, max_pitch, min_pitch, yaw]
            if numbers.iter().all(|n| n.is_finite())
                && roll > 0.0
                && yaw > 0.0
                && max_pitch > min_pitch =>
        {
            Ok(GimbalLimits::from_px4(roll, max_pitch, min_pitch, yaw))
        }
        _ => Err(invalid()),
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
                "--step-ms" => {
                    let value = value()?;
                    let step: u32 = parse(flag, value.clone(), "1, 2, or 4")?;
                    if !STEP_MS_CHOICES.contains(&step) {
                        return Err(ArgsError::Invalid {
                            flag: flag.clone(),
                            value,
                            reason: "1, 2, or 4",
                        });
                    }
                    args.step_ms = step;
                }
                "--truth-addr" => {
                    args.truth_addr = Some(parse(flag, value()?, "a socket address")?)
                }
                "--gimbal" => args.gimbal = Some(gimbal_limits(flag, value()?)?),
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
         [--realtime-factor F] [--lockstep-timeout-s S] \
         [--gimbal RANGE_ROLL,MAX_PITCH,MIN_PITCH,RANGE_YAW] [--truth-addr HOST:PORT] [--step-ms 1|2|4]",
        AIRCRAFT_NAMES.join("|")
    )
}

#[cfg(test)]
mod tests;

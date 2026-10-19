//! Command-line options. Defaults put the quad at the runway 04 threshold
//! of Solberg-Hunterdon Airport (N51) and talk to a Betaflight SITL on
//! loopback.

use std::net::SocketAddr;

use pilotage_multirotor_fdm::Origin;

use crate::sitl::SitlConfig;

/// Parsed options.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Args {
    /// Where Betaflight sends motor commands (its fixed UDP 9002).
    pub(crate) motor_listen: SocketAddr,
    /// Where Betaflight reads state (its fixed UDP 9003).
    pub(crate) state_target: SocketAddr,
    /// The local frame's geodetic origin.
    pub(crate) origin: Origin,
    /// Spawn offset north of the origin, m.
    pub(crate) north_m: f64,
    /// Spawn offset east of the origin, m.
    pub(crate) east_m: f64,
    /// Spawn heading, radians clockwise from north.
    pub(crate) heading_rad: f64,
    /// Physics and state-packet rate, Hz.
    pub(crate) rate_hz: f64,
    /// The Betaflight SITL to run as a child, when the sidecar owns it.
    pub(crate) betaflight: Option<SitlConfig>,
}

/// A malformed command line.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ArgsError {
    #[error("{flag} requires a value")]
    MissingValue { flag: String },
    #[error("{flag} value {value:?} is not valid: {reason}")]
    Invalid {
        flag: String,
        value: String,
        reason: &'static str,
    },
    #[error("unrecognized argument {0:?}")]
    Unrecognized(String),
}

impl Default for Args {
    fn default() -> Self {
        Self {
            motor_listen: SocketAddr::from(([127, 0, 0, 1], 9_002)),
            state_target: SocketAddr::from(([127, 0, 0, 1], 9_003)),
            origin: Origin {
                lat_deg: 40.577_186_1,
                lon_deg: -74.738_191_8,
                alt_m: 56.3,
            },
            north_m: 0.0,
            east_m: 0.0,
            heading_rad: 0.0,
            rate_hz: 1_000.0,
            betaflight: None,
        }
    }
}

impl Args {
    /// Parses `argv` (without the program name).
    pub(crate) fn parse(argv: &[String]) -> Result<Self, ArgsError> {
        let mut args = Self::default();
        let mut sitl = SitlFlags::default();
        let mut iter = argv.iter();
        while let Some(flag) = iter.next() {
            let mut value = || {
                iter.next()
                    .cloned()
                    .ok_or(ArgsError::MissingValue { flag: flag.clone() })
            };
            match flag.as_str() {
                "--motor-listen" => args.motor_listen = parse(flag, value()?, "a socket address")?,
                "--state-target" => args.state_target = parse(flag, value()?, "a socket address")?,
                "--origin-lat" => args.origin.lat_deg = finite(flag, value()?)?,
                "--origin-lon" => args.origin.lon_deg = finite(flag, value()?)?,
                "--origin-alt" => args.origin.alt_m = finite(flag, value()?)?,
                "--north" => args.north_m = finite(flag, value()?)?,
                "--east" => args.east_m = finite(flag, value()?)?,
                "--heading-deg" => args.heading_rad = finite(flag, value()?)?.to_radians(),
                "--rate-hz" => {
                    let rate = finite(flag, value()?)?;
                    if !(100.0..=4_000.0).contains(&rate) {
                        return Err(ArgsError::Invalid {
                            flag: flag.clone(),
                            value: rate.to_string(),
                            reason: "the rate must be 100 to 4000 Hz",
                        });
                    }
                    args.rate_hz = rate;
                }
                "--betaflight" => sitl.executable = Some(value()?.into()),
                "--betaflight-dir" => sitl.work_dir = Some(value()?.into()),
                "--betaflight-config" => sitl.config = Some(value()?.into()),
                other => return Err(ArgsError::Unrecognized(other.to_owned())),
            }
        }
        args.betaflight = sitl.into_config()?;
        Ok(args)
    }
}

/// The `--betaflight*` flags before they are checked together.
#[derive(Default)]
struct SitlFlags {
    executable: Option<std::path::PathBuf>,
    work_dir: Option<std::path::PathBuf>,
    config: Option<std::path::PathBuf>,
}

impl SitlFlags {
    fn into_config(self) -> Result<Option<SitlConfig>, ArgsError> {
        let Some(executable) = self.executable else {
            if self.work_dir.is_some() || self.config.is_some() {
                return Err(ArgsError::MissingValue {
                    flag: "--betaflight (required by --betaflight-dir and --betaflight-config)"
                        .to_owned(),
                });
            }
            return Ok(None);
        };
        Ok(Some(SitlConfig {
            executable,
            work_dir: self.work_dir.unwrap_or_else(|| ".".into()),
            config: self.config,
        }))
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

/// A finite number: Betaflight SITL is built with fast-math, which cannot
/// see a NaN, so none may reach a packet.
fn finite(flag: &str, value: String) -> Result<f64, ArgsError> {
    let number: f64 = parse(flag, value.clone(), "a number")?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(ArgsError::Invalid {
            flag: flag.to_owned(),
            value,
            reason: "the value must be finite",
        })
    }
}

#[cfg(test)]
mod tests;

//! Session-host arguments. Remote binds require an explicit admission secret.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

/// Which vehicle adapter the host embeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdapterKind {
    /// The deterministic headless reference adapter (default; 1a behavior).
    #[default]
    Reference,
    /// The real Gazebo diff-drive adapter driven through the sidecar bridge.
    Gazebo,
    /// The telemetry-only Aviate flight-controller adapter over MAVLink
    /// (ADR-0018).
    Aviate,
    /// The PX4 adapter over standard MAVLink: telemetry plus offboard
    /// velocity control (ADR-0018).
    Px4,
    /// The ArduCopter adapter over MAVLink: telemetry plus GUIDED body
    /// velocity control (ADR-0018).
    ArduPilot,
    /// The Betaflight adapter over MSP: telemetry plus a host-side velocity
    /// loop flying ANGLE mode.
    Betaflight,
    /// The DJI USB bench adapter. Flight and positioning are not implemented.
    Dji,
    /// No vehicle: a coordinator whose mission lane works through member
    /// hosts (ADR-0028, ADR-0052).
    Coordinator,
    /// The simulated multicopter on the DJI control path, with an
    /// aerial-imagery gimbal camera.
    SimMulticopter,
}

impl AdapterKind {
    /// The adapter's name on the command line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Gazebo => "gazebo",
            Self::Aviate => "aviate",
            Self::Px4 => "px4",
            Self::ArduPilot => "ardupilot",
            Self::Betaflight => "betaflight",
            Self::Dji => "dji",
            Self::Coordinator => "coordinator",
            Self::SimMulticopter => "sim-multicopter",
        }
    }

    /// Whether the adapter drives a simulation by its nature. Adapters that
    /// can drive either a simulation or a physical vehicle report `false`;
    /// a lane then takes `PILOTAGE_LANE_SIMULATED`.
    #[must_use]
    pub const fn is_simulator(self) -> bool {
        matches!(self, Self::Reference | Self::Gazebo | Self::SimMulticopter)
    }
}

/// Parsed command-line configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliArgs {
    /// Port to bind. `0` asks the OS for an ephemeral port.
    pub port: u16,
    /// Interface address. Defaults to `127.0.0.1`.
    pub bind_ip: IpAddr,
    /// Absolute path to a hexadecimal admission secret.
    pub join_token_file: Option<PathBuf>,
    /// Which vehicle adapter to embed.
    pub adapter: AdapterKind,
}

/// An error parsing command-line arguments.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliError {
    /// A network option has no following value.
    #[error("{0} requires a value")]
    MissingNetworkValue(String),
    /// The bind address is not an IP address.
    #[error("invalid --bind address: {0}")]
    InvalidBind(#[source] std::net::AddrParseError),
    /// `--port` was given a value that does not parse as a `u16`.
    #[error("invalid --port value {value:?}: {source}")]
    InvalidPort {
        /// The raw string the user supplied.
        value: String,
        /// The underlying parse failure.
        #[source]
        source: core::num::ParseIntError,
    },
    /// `--port` was given with no following value.
    #[error("--port requires a value")]
    MissingPortValue,
    /// `--adapter` was given with no following value.
    #[error("--adapter requires a value")]
    MissingAdapterValue,
    /// `--adapter` was given a value outside the known adapter set.
    #[error(
        "invalid --adapter value {0:?} (expected reference, gazebo, aviate, px4, ardupilot, betaflight, dji, or sim-multicopter)"
    )]
    InvalidAdapter(String),
    /// An argument was not recognized.
    #[error("unrecognized argument: {0}")]
    Unrecognized(String),
}

/// Parses `args` (excluding the program name) into [`CliArgs`].
///
/// # Errors
///
/// Returns [`CliError`] for a malformed, missing, or unrecognized argument.
pub fn parse_args(args: &[String]) -> Result<CliArgs, CliError> {
    let mut port: u16 = 0;
    let mut adapter = AdapterKind::default();
    let mut bind_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut join_token_file = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--bind" => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::MissingNetworkValue(arg.clone()))?;
                bind_ip = value.parse().map_err(CliError::InvalidBind)?;
            }
            "--join-token-file" => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::MissingNetworkValue(arg.clone()))?;
                join_token_file = Some(PathBuf::from(value));
            }
            "--port" => {
                let value = iter.next().ok_or(CliError::MissingPortValue)?;
                port = value.parse().map_err(|source| CliError::InvalidPort {
                    value: value.clone(),
                    source,
                })?;
            }
            "--adapter" => {
                let value = iter.next().ok_or(CliError::MissingAdapterValue)?;
                adapter = match value.as_str() {
                    "reference" => AdapterKind::Reference,
                    "gazebo" => AdapterKind::Gazebo,
                    "aviate" => AdapterKind::Aviate,
                    "px4" => AdapterKind::Px4,
                    "ardupilot" => AdapterKind::ArduPilot,
                    "betaflight" => AdapterKind::Betaflight,
                    "dji" => AdapterKind::Dji,
                    "coordinator" => AdapterKind::Coordinator,
                    "sim-multicopter" => AdapterKind::SimMulticopter,
                    other => return Err(CliError::InvalidAdapter(other.to_owned())),
                };
            }
            other => return Err(CliError::Unrecognized(other.to_owned())),
        }
    }
    Ok(CliArgs {
        port,
        bind_ip,
        join_token_file,
        adapter,
    })
}

#[cfg(test)]
mod tests;

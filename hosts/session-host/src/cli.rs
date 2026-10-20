//! Minimal argument parsing for the session-host binary: `--port <PORT>` and
//! `--adapter reference|gazebo|aviate|px4|ardupilot|betaflight|dji`, defaulting to
//! port `0` (ephemeral, loopback-only bind) and the reference adapter.

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
}

/// Parsed command-line configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliArgs {
    /// Port to bind on `127.0.0.1`. `0` asks the OS for an ephemeral port.
    pub port: u16,
    /// Which vehicle adapter to embed.
    pub adapter: AdapterKind,
}

/// An error parsing command-line arguments.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliError {
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
        "invalid --adapter value {0:?} (expected reference, gazebo, aviate, px4, ardupilot, betaflight, or dji)"
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
/// Returns [`CliError`] for a malformed or unrecognized `--port` argument.
pub fn parse_args(args: &[String]) -> Result<CliArgs, CliError> {
    let mut port: u16 = 0;
    let mut adapter = AdapterKind::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
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
                    other => return Err(CliError::InvalidAdapter(other.to_owned())),
                };
            }
            other => return Err(CliError::Unrecognized(other.to_owned())),
        }
    }
    Ok(CliArgs { port, adapter })
}

#[cfg(test)]
mod tests;

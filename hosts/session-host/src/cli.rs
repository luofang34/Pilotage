//! Minimal argument parsing for the session-host binary: `--port <PORT>` and
//! `--adapter reference|gazebo|aviate`, defaulting to port `0` (ephemeral,
//! loopback-only bind) and the reference adapter.

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
    /// `--adapter` was given a value other than `reference`, `gazebo`,
    /// or `aviate`.
    #[error("invalid --adapter value {0:?} (expected reference, gazebo, or aviate)")]
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
                    other => return Err(CliError::InvalidAdapter(other.to_owned())),
                };
            }
            other => return Err(CliError::Unrecognized(other.to_owned())),
        }
    }
    Ok(CliArgs { port, adapter })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{AdapterKind, CliError, parse_args};

    #[test]
    fn no_args_defaults_to_ephemeral_port() {
        let parsed = parse_args(&[]).expect("empty args parse");
        assert_eq!(parsed.port, 0);
        assert_eq!(parsed.adapter, AdapterKind::Reference);
    }

    #[test]
    fn explicit_port_parses() {
        let args = vec!["--port".to_owned(), "4433".to_owned()];
        let parsed = parse_args(&args).expect("valid port parses");
        assert_eq!(parsed.port, 4433);
    }

    #[test]
    fn gazebo_adapter_parses() {
        let args = vec!["--adapter".to_owned(), "gazebo".to_owned()];
        let parsed = parse_args(&args).expect("gazebo adapter parses");
        assert_eq!(parsed.adapter, AdapterKind::Gazebo);
    }

    #[test]
    fn reference_adapter_parses() {
        let args = vec!["--adapter".to_owned(), "reference".to_owned()];
        let parsed = parse_args(&args).expect("reference adapter parses");
        assert_eq!(parsed.adapter, AdapterKind::Reference);
    }

    #[test]
    fn aviate_adapter_parses() {
        let args = vec!["--adapter".to_owned(), "aviate".to_owned()];
        let parsed = parse_args(&args).expect("aviate adapter parses");
        assert_eq!(parsed.adapter, AdapterKind::Aviate);
    }

    #[test]
    fn unknown_adapter_is_an_error() {
        let args = vec!["--adapter".to_owned(), "unreal".to_owned()];
        assert_eq!(
            parse_args(&args),
            Err(CliError::InvalidAdapter("unreal".to_owned()))
        );
    }

    #[test]
    fn missing_adapter_value_is_an_error() {
        let args = vec!["--adapter".to_owned()];
        assert_eq!(parse_args(&args), Err(CliError::MissingAdapterValue));
    }

    #[test]
    fn missing_port_value_is_an_error() {
        let args = vec!["--port".to_owned()];
        assert_eq!(parse_args(&args), Err(CliError::MissingPortValue));
    }

    #[test]
    fn non_numeric_port_is_an_error() {
        let args = vec!["--port".to_owned(), "not-a-port".to_owned()];
        assert!(matches!(
            parse_args(&args),
            Err(CliError::InvalidPort { .. })
        ));
    }

    #[test]
    fn unrecognized_argument_is_an_error() {
        let args = vec!["--bogus".to_owned()];
        assert_eq!(
            parse_args(&args),
            Err(CliError::Unrecognized("--bogus".to_owned()))
        );
    }
}

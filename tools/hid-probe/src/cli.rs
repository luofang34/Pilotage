//! Hand-rolled argument parsing for `hid-probe` (no `clap`, per task scope).

use std::path::PathBuf;

use crate::error::ProbeError;

/// The parsed subcommand and its arguments.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `list`: enumerate all connected HID devices.
    List,
    /// `read --seconds N`: open the target device and print reports for
    /// `seconds` wall-clock seconds worth of read-loop iterations.
    Read {
        /// How many seconds to keep reading input reports.
        seconds: u64,
    },
    /// `capture --seconds N --out PATH`: same as `read`, but records every
    /// report to a JSON file at `PATH` instead of printing it live.
    Capture {
        /// How many seconds to keep reading input reports.
        seconds: u64,
        /// Output path for the captured JSON.
        out: PathBuf,
    },
}

/// Parses `std::env::args()` (already stripped of argv\[0\]) into a
/// [`Command`].
///
/// # Errors
///
/// Returns [`ProbeError::Usage`] for an empty, unknown, or malformed
/// argument list.
pub fn parse_args(args: &[String]) -> Result<Command, ProbeError> {
    let (name, rest) = args.split_first().ok_or_else(|| ProbeError::Usage {
        message: "expected a subcommand: list | read | capture".to_string(),
    })?;
    match name.as_str() {
        "list" => Ok(Command::List),
        "read" => parse_read(rest),
        "capture" => parse_capture(rest),
        other => Err(ProbeError::Usage {
            message: format!("unknown subcommand '{other}'"),
        }),
    }
}

/// Parses `--seconds N` flags for the `read` subcommand.
fn parse_read(args: &[String]) -> Result<Command, ProbeError> {
    let seconds = flag_u64(args, "--seconds")?;
    Ok(Command::Read { seconds })
}

/// Parses `--seconds N --out PATH` flags for the `capture` subcommand.
fn parse_capture(args: &[String]) -> Result<Command, ProbeError> {
    let seconds = flag_u64(args, "--seconds")?;
    let out = flag_str(args, "--out")?;
    Ok(Command::Capture {
        seconds,
        out: PathBuf::from(out),
    })
}

/// Finds `--name VALUE` in `args` and parses `VALUE` as `u64`.
fn flag_u64(args: &[String], name: &str) -> Result<u64, ProbeError> {
    let raw = flag_str(args, name)?;
    raw.parse::<u64>().map_err(|source| ProbeError::Usage {
        message: format!("{name} expects an integer, got '{raw}': {source}"),
    })
}

/// Finds `--name VALUE` in `args` and returns `VALUE` as a `&str`.
fn flag_str<'a>(args: &'a [String], name: &str) -> Result<&'a str, ProbeError> {
    let position = args
        .iter()
        .position(|arg| arg == name)
        .ok_or_else(|| ProbeError::Usage {
            message: format!("missing required flag {name}"),
        })?;
    args.get(position + 1)
        .map(String::as_str)
        .ok_or_else(|| ProbeError::Usage {
            message: format!("{name} requires a value"),
        })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Command, parse_args};

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn parses_list() {
        assert_eq!(parse_args(&args(&["list"])).expect("list"), Command::List);
    }

    #[test]
    fn parses_read_with_seconds() {
        let command = parse_args(&args(&["read", "--seconds", "3"])).expect("read");
        assert_eq!(command, Command::Read { seconds: 3 });
    }

    #[test]
    fn parses_capture_with_seconds_and_out() {
        let command =
            parse_args(&args(&["capture", "--seconds", "5", "--out", "out.json"])).expect("cap");
        assert_eq!(
            command,
            Command::Capture {
                seconds: 5,
                out: "out.json".into(),
            }
        );
    }

    #[test]
    fn rejects_empty_args() {
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn rejects_unknown_subcommand() {
        assert!(parse_args(&args(&["bogus"])).is_err());
    }

    #[test]
    fn rejects_read_missing_seconds() {
        assert!(parse_args(&args(&["read"])).is_err());
    }

    #[test]
    fn rejects_non_integer_seconds() {
        assert!(parse_args(&args(&["read", "--seconds", "soon"])).is_err());
    }

    #[test]
    fn rejects_capture_missing_out() {
        assert!(parse_args(&args(&["capture", "--seconds", "5"])).is_err());
    }
}

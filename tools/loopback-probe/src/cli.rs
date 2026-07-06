//! Hand-rolled argument parsing for `loopback-probe` (no `clap`, matching
//! `tools/hid-probe`'s established pattern for this workspace).

use crate::error::ProbeError;

/// Parsed command-line configuration for one probe run.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    /// WebTransport URL of the session host to connect to, e.g.
    /// `https://127.0.0.1:4433`.
    pub url: String,
    /// Whether the caller passed `--insecure-loopback`. Required to skip
    /// server-certificate verification; refused by `validate_loopback_url`
    /// unless `url`'s host is a loopback address, so a skipped-verification
    /// connection can never be pointed at a non-local host by accident.
    pub insecure_loopback: bool,
    /// Whether to drive control input from the RadioMaster Pocket via HID
    /// instead of the synthetic sine-wave generator.
    pub hid: bool,
    /// How many wall-clock seconds to run the control/telemetry loop.
    pub seconds: u64,
    /// Control-frame send rate in Hz.
    pub rate: u64,
    /// Whether to send the scripted forward-then-arc drive pattern instead of
    /// the default synthetic sine-wave generator. Takes precedence over
    /// `--hid` if both are somehow given, since `--drive` is this tool's
    /// deliberate "move the real vehicle" demo mode.
    pub drive: bool,
    /// If set, save the first, a middle, and the last decoded video frame as
    /// PNG files under this directory.
    pub save_frames: Option<String>,
}

const DEFAULT_SECONDS: u64 = 3;
const DEFAULT_RATE: u64 = 100;

/// Parses `std::env::args()` (already stripped of argv\[0\]) into [`Args`].
///
/// # Errors
///
/// Returns [`ProbeError::Usage`] if `--url` is missing, or any flag value
/// fails to parse. Returns [`ProbeError::InvalidUrl`] if `--insecure-loopback`
/// is set without a required companion, though URL loopback-scope validation
/// itself lives in `transport` (it needs a parsed URL, not raw args).
pub fn parse_args(args: &[String]) -> Result<Args, ProbeError> {
    let url = require_flag(args, "--url")?.to_string();
    let insecure_loopback = has_flag(args, "--insecure-loopback");
    if !insecure_loopback {
        return Err(ProbeError::Usage {
            message: "--insecure-loopback is required (this tool only talks to loopback hosts \
                      and skips certificate verification; passing the flag is how you attest \
                      that's understood)"
                .to_string(),
        });
    }
    let hid = has_flag(args, "--hid");
    let drive = has_flag(args, "--drive");
    let seconds = optional_u64(args, "--seconds")?.unwrap_or(DEFAULT_SECONDS);
    let rate = optional_u64(args, "--rate")?.unwrap_or(DEFAULT_RATE);
    if rate == 0 {
        return Err(ProbeError::Usage {
            message: "--rate must be greater than zero".to_string(),
        });
    }
    let save_frames = optional_flag(args, "--save-frames")?;
    Ok(Args {
        url,
        insecure_loopback,
        hid,
        seconds,
        rate,
        drive,
        save_frames,
    })
}

/// Returns whether boolean flag `name` is present anywhere in `args`.
fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

/// Finds `--name VALUE` in `args` and returns `VALUE`, erroring if absent.
fn require_flag<'a>(args: &'a [String], name: &str) -> Result<&'a str, ProbeError> {
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

/// Finds `--name VALUE` in `args` and returns `VALUE` as an owned `String`,
/// returning `None` if the flag was not supplied at all.
fn optional_flag(args: &[String], name: &str) -> Result<Option<String>, ProbeError> {
    let Some(position) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    let value = args.get(position + 1).ok_or_else(|| ProbeError::Usage {
        message: format!("{name} requires a value"),
    })?;
    Ok(Some(value.clone()))
}

/// Finds `--name VALUE` in `args` and parses it as `u64`, returning `None`
/// if the flag was not supplied at all.
fn optional_u64(args: &[String], name: &str) -> Result<Option<u64>, ProbeError> {
    let Some(position) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    let raw = args.get(position + 1).ok_or_else(|| ProbeError::Usage {
        message: format!("{name} requires a value"),
    })?;
    let value = raw.parse::<u64>().map_err(|source| ProbeError::Usage {
        message: format!("{name} expects an integer, got '{raw}': {source}"),
    })?;
    Ok(Some(value))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Args, parse_args};

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn parses_minimal_required_flags_with_defaults() {
        let parsed = parse_args(&args(&[
            "--url",
            "https://127.0.0.1:4433",
            "--insecure-loopback",
        ]))
        .expect("parses");
        assert_eq!(
            parsed,
            Args {
                url: "https://127.0.0.1:4433".to_string(),
                insecure_loopback: true,
                hid: false,
                seconds: 3,
                rate: 100,
                drive: false,
                save_frames: None,
            }
        );
    }

    #[test]
    fn parses_all_flags() {
        let parsed = parse_args(&args(&[
            "--url",
            "https://127.0.0.1:4433",
            "--insecure-loopback",
            "--hid",
            "--drive",
            "--seconds",
            "10",
            "--rate",
            "50",
            "--save-frames",
            "/tmp/frames",
        ]))
        .expect("parses");
        assert!(parsed.hid);
        assert!(parsed.drive);
        assert_eq!(parsed.seconds, 10);
        assert_eq!(parsed.rate, 50);
        assert_eq!(parsed.save_frames.as_deref(), Some("/tmp/frames"));
    }

    #[test]
    fn rejects_missing_url() {
        assert!(parse_args(&args(&["--insecure-loopback"])).is_err());
    }

    #[test]
    fn rejects_missing_insecure_loopback_flag() {
        assert!(parse_args(&args(&["--url", "https://127.0.0.1:4433"])).is_err());
    }

    #[test]
    fn rejects_zero_rate() {
        let err = parse_args(&args(&[
            "--url",
            "https://127.0.0.1:4433",
            "--insecure-loopback",
            "--rate",
            "0",
        ]));
        assert!(err.is_err());
    }

    #[test]
    fn rejects_non_integer_seconds() {
        let err = parse_args(&args(&[
            "--url",
            "https://127.0.0.1:4433",
            "--insecure-loopback",
            "--seconds",
            "soon",
        ]));
        assert!(err.is_err());
    }
}

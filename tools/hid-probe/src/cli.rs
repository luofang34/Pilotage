//! Strict command-line argument parsing for `hid-probe`.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::error::ProbeError;

const MAX_READ_SECONDS: u64 = 3_600;
const MAX_SEGMENT_SECONDS: u64 = 120;
const MAX_LOGICAL_NAME_BYTES: usize = 64;

/// The parsed subcommand and its arguments.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Enumerate connected HID devices.
    List,
    /// Read target-device reports for a fixed interval.
    Read {
        /// Seconds to read.
        seconds: u64,
    },
    /// Record guided idle and named-control movement segments.
    Capture {
        /// Seconds for the idle segment.
        idle_seconds: u64,
        /// Seconds for each movement segment.
        movement_seconds: u64,
        /// Named controls in guided order.
        logical_axes: Vec<String>,
        /// Output capture path.
        out: PathBuf,
    },
    /// Create a calibration candidate from one capture.
    Characterize {
        /// Trusted source-axis contract path.
        contract: PathBuf,
        /// Source capture path.
        capture: PathBuf,
        /// Baseline device profile path.
        profile: PathBuf,
        /// Output candidate path.
        out: PathBuf,
    },
    /// Promote one candidate after explicit digest confirmation.
    Promote {
        /// Trusted source-axis contract path.
        contract: PathBuf,
        /// Exact source capture path.
        capture: PathBuf,
        /// Candidate path.
        candidate: PathBuf,
        /// Baseline device profile path.
        profile: PathBuf,
        /// Output profile path.
        out: PathBuf,
        /// Source digest reviewed by the operator.
        confirmed_source_digest: String,
        /// Canonical candidate digest reviewed by the operator.
        confirmed_candidate_digest: String,
    },
}

/// Parses arguments after the program name.
///
/// # Errors
///
/// Returns [`ProbeError::Usage`] for an empty, unknown, duplicate, or
/// malformed argument list.
pub fn parse_args(args: &[String]) -> Result<Command, ProbeError> {
    let (name, rest) = args
        .split_first()
        .ok_or_else(|| usage("expected a subcommand"))?;
    match name.as_str() {
        "list" => parse_list(rest),
        "read" => parse_read(rest),
        "capture" => parse_capture(rest),
        "characterize" => parse_characterize(rest),
        "promote" => parse_promote(rest),
        other => Err(usage(&format!("unknown subcommand '{other}'"))),
    }
}

fn parse_list(args: &[String]) -> Result<Command, ProbeError> {
    if args.is_empty() {
        Ok(Command::List)
    } else {
        Err(usage("list takes no flags"))
    }
}

fn parse_read(args: &[String]) -> Result<Command, ProbeError> {
    validate_flags(args, &["--seconds"])?;
    Ok(Command::Read {
        seconds: bounded_u64(args, "--seconds", MAX_READ_SECONDS)?,
    })
}

fn parse_capture(args: &[String]) -> Result<Command, ProbeError> {
    validate_flags(
        args,
        &["--idle-seconds", "--movement-seconds", "--axes", "--out"],
    )?;
    let logical_axes = parse_axes(flag(args, "--axes")?)?;
    Ok(Command::Capture {
        idle_seconds: bounded_u64(args, "--idle-seconds", MAX_SEGMENT_SECONDS)?,
        movement_seconds: bounded_u64(args, "--movement-seconds", MAX_SEGMENT_SECONDS)?,
        logical_axes,
        out: flag(args, "--out")?.into(),
    })
}

fn parse_characterize(args: &[String]) -> Result<Command, ProbeError> {
    validate_flags(args, &["--contract", "--capture", "--profile", "--out"])?;
    Ok(Command::Characterize {
        contract: flag(args, "--contract")?.into(),
        capture: flag(args, "--capture")?.into(),
        profile: flag(args, "--profile")?.into(),
        out: flag(args, "--out")?.into(),
    })
}

fn parse_promote(args: &[String]) -> Result<Command, ProbeError> {
    validate_flags(
        args,
        &[
            "--contract",
            "--capture",
            "--candidate",
            "--profile",
            "--out",
            "--confirm-source-digest",
            "--confirm-candidate-digest",
        ],
    )?;
    Ok(Command::Promote {
        contract: flag(args, "--contract")?.into(),
        capture: flag(args, "--capture")?.into(),
        candidate: flag(args, "--candidate")?.into(),
        profile: flag(args, "--profile")?.into(),
        out: flag(args, "--out")?.into(),
        confirmed_source_digest: flag(args, "--confirm-source-digest")?.to_owned(),
        confirmed_candidate_digest: flag(args, "--confirm-candidate-digest")?.to_owned(),
    })
}

fn validate_flags(args: &[String], allowed: &[&str]) -> Result<(), ProbeError> {
    let (pairs, remainder) = args.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(usage("each flag needs one value"));
    }
    let mut seen = HashSet::new();
    for pair in pairs {
        if !allowed.contains(&pair[0].as_str()) {
            return Err(usage(&format!("unknown flag {}", pair[0])));
        }
        if !seen.insert(pair[0].as_str()) {
            return Err(usage(&format!("duplicate flag {}", pair[0])));
        }
    }
    if seen.len() != allowed.len() {
        return Err(usage("one or more required flags are missing"));
    }
    Ok(())
}

fn bounded_u64(args: &[String], name: &str, maximum: u64) -> Result<u64, ProbeError> {
    let raw = flag(args, name)?;
    let value = raw.parse::<u64>().map_err(|source| {
        usage(&format!(
            "{name} expects a positive integer, got '{raw}': {source}"
        ))
    })?;
    if value == 0 {
        return Err(usage(&format!("{name} must be greater than zero")));
    }
    if value > maximum {
        return Err(usage(&format!("{name} must not exceed {maximum}")));
    }
    Ok(value)
}

fn parse_axes(raw: &str) -> Result<Vec<String>, ProbeError> {
    let axes = raw
        .split(',')
        .map(str::trim)
        .filter(|axis| !axis.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if axes.is_empty() || axes.len() > crate::device::AXIS_COUNT {
        return Err(usage("--axes count is outside the target device contract"));
    }
    let mut seen = HashSet::new();
    for axis in &axes {
        if axis.len() > MAX_LOGICAL_NAME_BYTES || pilotage_input::axis_id_for_name(axis).is_err() {
            return Err(usage(&format!("invalid logical axis {axis}")));
        }
        if !seen.insert(axis.as_str()) {
            return Err(usage(&format!("duplicate logical axis {axis}")));
        }
    }
    Ok(axes)
}

fn flag<'a>(args: &'a [String], name: &str) -> Result<&'a str, ProbeError> {
    args.as_chunks::<2>()
        .0
        .iter()
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
        .ok_or_else(|| usage(&format!("missing required flag {name}")))
}

fn usage(message: &str) -> ProbeError {
    ProbeError::Usage {
        message: message.to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Command, parse_args};

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn parses_guided_capture() {
        let command = parse_args(&args(&[
            "capture",
            "--idle-seconds",
            "5",
            "--movement-seconds",
            "4",
            "--axes",
            "roll,pitch,yaw",
            "--out",
            "capture.json",
        ]))
        .expect("capture");
        assert_eq!(
            command,
            Command::Capture {
                idle_seconds: 5,
                movement_seconds: 4,
                logical_axes: vec!["roll".to_owned(), "pitch".to_owned(), "yaw".to_owned()],
                out: "capture.json".into(),
            }
        );
    }

    #[test]
    fn parses_explicit_promotion_confirmation() {
        let digest = "a".repeat(64);
        let command = parse_args(&args(&[
            "promote",
            "--contract",
            "contract.json",
            "--capture",
            "capture.json",
            "--candidate",
            "candidate.json",
            "--profile",
            "device.json",
            "--out",
            "accepted.json",
            "--confirm-source-digest",
            &digest,
            "--confirm-candidate-digest",
            &digest,
        ]))
        .expect("promote");
        assert!(matches!(
            command,
            Command::Promote {
                confirmed_source_digest,
                confirmed_candidate_digest,
                ..
            } if confirmed_source_digest == digest && confirmed_candidate_digest == digest
        ));
    }

    #[test]
    fn rejects_missing_duplicate_and_unknown_flags() {
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&args(&["read", "--seconds", "0"])).is_err());
        assert!(parse_args(&args(&["read", "--seconds", "3601"])).is_err());
        assert!(parse_args(&args(&["read", "--wait", "1"])).is_err());
        assert!(parse_args(&args(&["read", "--seconds", "1", "--seconds", "2"])).is_err());
        assert!(
            parse_args(&args(&[
                "capture",
                "--idle-seconds",
                "5",
                "--movement-seconds",
                "4",
                "--axes",
                "roll,roll",
                "--out",
                "capture.json",
            ]))
            .is_err()
        );
        assert!(
            parse_args(&args(&[
                "promote",
                "--contract",
                "contract.json",
                "--capture",
                "capture.json",
                "--candidate",
                "candidate.json",
                "--profile",
                "profile.json",
                "--out",
                "out.json",
                "--confirm-source-digest",
                &"a".repeat(64),
            ]))
            .is_err()
        );
    }
}

//! Strict command-line argument parsing for `hid-probe`.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::error::ProbeError;

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
        /// Source capture path.
        capture: PathBuf,
        /// Baseline device profile path.
        profile: PathBuf,
        /// Output candidate path.
        out: PathBuf,
    },
    /// Promote one candidate after explicit digest confirmation.
    Promote {
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
        seconds: positive_u64(args, "--seconds")?,
    })
}

fn parse_capture(args: &[String]) -> Result<Command, ProbeError> {
    validate_flags(
        args,
        &["--idle-seconds", "--movement-seconds", "--axes", "--out"],
    )?;
    let logical_axes = flag(args, "--axes")?
        .split(',')
        .map(str::trim)
        .filter(|axis| !axis.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if logical_axes.is_empty() {
        return Err(usage("--axes needs at least one comma-separated name"));
    }
    Ok(Command::Capture {
        idle_seconds: positive_u64(args, "--idle-seconds")?,
        movement_seconds: positive_u64(args, "--movement-seconds")?,
        logical_axes,
        out: flag(args, "--out")?.into(),
    })
}

fn parse_characterize(args: &[String]) -> Result<Command, ProbeError> {
    validate_flags(args, &["--capture", "--profile", "--out"])?;
    Ok(Command::Characterize {
        capture: flag(args, "--capture")?.into(),
        profile: flag(args, "--profile")?.into(),
        out: flag(args, "--out")?.into(),
    })
}

fn parse_promote(args: &[String]) -> Result<Command, ProbeError> {
    validate_flags(
        args,
        &[
            "--candidate",
            "--profile",
            "--out",
            "--confirm-source-digest",
            "--confirm-candidate-digest",
        ],
    )?;
    Ok(Command::Promote {
        candidate: flag(args, "--candidate")?.into(),
        profile: flag(args, "--profile")?.into(),
        out: flag(args, "--out")?.into(),
        confirmed_source_digest: flag(args, "--confirm-source-digest")?.to_owned(),
        confirmed_candidate_digest: flag(args, "--confirm-candidate-digest")?.to_owned(),
    })
}

fn validate_flags(args: &[String], allowed: &[&str]) -> Result<(), ProbeError> {
    let chunks = args.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return Err(usage("each flag needs one value"));
    }
    let mut seen = HashSet::new();
    for pair in chunks {
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

fn positive_u64(args: &[String], name: &str) -> Result<u64, ProbeError> {
    let raw = flag(args, name)?;
    let value = raw.parse::<u64>().map_err(|source| {
        usage(&format!(
            "{name} expects a positive integer, got '{raw}': {source}"
        ))
    })?;
    if value == 0 {
        return Err(usage(&format!("{name} must be greater than zero")));
    }
    Ok(value)
}

fn flag<'a>(args: &'a [String], name: &str) -> Result<&'a str, ProbeError> {
    args.chunks_exact(2)
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
        assert!(parse_args(&args(&["read", "--wait", "1"])).is_err());
        assert!(parse_args(&args(&["read", "--seconds", "1", "--seconds", "2"])).is_err());
        assert!(
            parse_args(&args(&[
                "promote",
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

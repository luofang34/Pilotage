//! Command-line options of the intent pilot.

use std::path::PathBuf;

use crate::error::PilotError;

/// Usage text, printed for `--help` and with every usage error.
pub(crate) const USAGE: &str = "\
intent-pilot: fly operator directives, read by a model, as a Pilotage control source

USAGE:
  intent-pilot --url <https://host:port/pilotage> --cert <sha256-hex> \\
               --scenario <file.json> --model-cmd <command> [--record <file.jsonl>] \\
               [--live] [--planar-truth]

  --url           Session host URL.
  --cert          SHA-256 of the host certificate, 64 hex digits. The host prints it
                  on its LISTENING line. The pilot pins it.
  --scenario      Scenario document: fixes, procedures, operator messages, expected result.
  --model-cmd     Shell command that starts a model adapter (JSON lines on stdin and stdout).
  --record        Run record path. Default: intent-pilot-run.jsonl
  --live          Also read operator messages from stdin, one on each line. The run gives
                  no verdict. `quit` or end of input brings the vehicle back and lands it.
  --planar-truth  Declare that the host runs the deterministic reference vehicle,
                  whose planar pose is simulator truth.

EXIT STATUS: 0 pass or live run ended, 1 fail, 2 inconclusive, 3 error.";

/// Parsed options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    /// Session host URL.
    pub url: String,
    /// Pinned SHA-256 of the host certificate.
    pub cert_sha256: [u8; 32],
    /// Scenario document path.
    pub scenario: PathBuf,
    /// Shell command that starts the model adapter.
    pub model_cmd: String,
    /// Run record path.
    pub record: PathBuf,
    /// True when operator messages also come from stdin.
    pub live: bool,
    /// True when the planar pose is simulator truth.
    pub planar_truth: bool,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Invocation {
    /// Print the usage text.
    Help,
    /// Fly.
    Fly(Options),
}

/// Parses the arguments that follow the program name.
pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Invocation, PilotError> {
    let usage = |detail: String| PilotError::Usage { detail };
    let (mut url, mut cert, mut scenario, mut model_cmd) = (None, None, None, None);
    let mut record = PathBuf::from("intent-pilot-run.jsonl");
    let (mut live, mut planar_truth) = (false, false);
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| usage(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            "--help" | "-h" => return Ok(Invocation::Help),
            "--live" => live = true,
            "--planar-truth" => planar_truth = true,
            "--url" => url = Some(value()?),
            "--cert" => cert = Some(parse_sha256(&value()?)?),
            "--scenario" => scenario = Some(PathBuf::from(value()?)),
            "--model-cmd" => model_cmd = Some(value()?),
            "--record" => record = PathBuf::from(value()?),
            other => return Err(usage(format!("unrecognized argument {other}"))),
        }
    }
    let missing = |name: &str| usage(format!("{name} is required"));
    Ok(Invocation::Fly(Options {
        url: url.ok_or_else(|| missing("--url"))?,
        cert_sha256: cert.ok_or_else(|| missing("--cert"))?,
        scenario: scenario.ok_or_else(|| missing("--scenario"))?,
        model_cmd: model_cmd.ok_or_else(|| missing("--model-cmd"))?,
        record,
        live,
        planar_truth,
    }))
}

fn parse_sha256(hex: &str) -> Result<[u8; 32], PilotError> {
    let invalid = || PilotError::Usage {
        detail: "--cert must be 64 hex digits".to_owned(),
    };
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(invalid());
    }
    let mut digest = [0_u8; 32];
    for (byte, pair) in digest.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
        let text = std::str::from_utf8(pair).map_err(|_| invalid())?;
        *byte = u8::from_str_radix(text, 16).map_err(|_| invalid())?;
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::{Invocation, parse};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|&arg| arg.to_owned()).collect()
    }

    #[test]
    fn a_complete_command_line_parses_and_pins_the_certificate() {
        let cert = "ab".repeat(32);
        let parsed = parse(args(&[
            "--url",
            "https://127.0.0.1:4433/pilotage",
            "--cert",
            &cert,
            "--scenario",
            "s.json",
            "--model-cmd",
            "python3 adapter.py",
            "--live",
        ]));
        let Ok(Invocation::Fly(options)) = parsed else {
            unreachable!("a complete command line parses: {parsed:?}");
        };
        assert_eq!(options.cert_sha256, [0xab; 32]);
        assert!(options.live && !options.planar_truth);
    }

    #[test]
    fn a_missing_or_short_certificate_is_refused() {
        let none = parse(args(&["--url", "u", "--scenario", "s", "--model-cmd", "c"]));
        assert!(none.is_err(), "the pilot has no unpinned mode");
        assert!(parse(args(&["--cert", "abcd"])).is_err());
    }
}

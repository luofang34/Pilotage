//! The command line.

use std::path::PathBuf;
use std::time::Duration;

/// Why the operator stopped.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OperatorError {
    #[error("usage: {0}")]
    Usage(String),
    #[error("cannot start the async runtime")]
    Runtime(#[source] std::io::Error),
    #[error("cannot read the objective {path:?}: {reason}")]
    Objective { path: PathBuf, reason: String },
    #[error("the host link failed")]
    Link(#[source] pilotage_client_link::LinkError),
    #[error("the host was not reached or did not answer: {0}")]
    Host(String),
}

/// The approval's envelope, as the operator states it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Envelope {
    pub max_height_m: f64,
    pub expires: Duration,
    pub refly_lines: u32,
    pub revise: bool,
    pub min_reserve: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    Propose(PathBuf),
    Approve(Vec<u8>, Envelope),
    Start(Vec<u8>),
    Hold,
    Resume,
    Abort,
    Watch,
    Run(PathBuf, Envelope),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Command {
    pub url: String,
    pub cert_sha256: [u8; 32],
    pub action: Action,
    /// How long `watch` and `run` wait at most.
    pub seconds: u64,
}

const USAGE: &str = "mission-operator --host URL --cert HEX (propose FILE | approve DIGEST [envelope] | start DIGEST | hold | resume | abort | watch | run FILE --approve [envelope]) [--seconds N]; envelope: --max-height M --expires-min N --refly N --no-revise --reserve F";

pub(crate) fn parse(args: &[String]) -> Result<Command, OperatorError> {
    let usage = || OperatorError::Usage(USAGE.to_owned());
    let mut url = None;
    let mut cert = None;
    let mut seconds = 3_600;
    let mut envelope = Envelope {
        max_height_m: 120.0,
        expires: Duration::from_secs(3_600),
        refly_lines: 4,
        revise: true,
        min_reserve: 0.2,
    };
    let mut approve = false;
    let mut positional = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || iter.next().cloned().ok_or_else(usage);
        match arg.as_str() {
            "--host" => url = Some(value()?),
            "--cert" => cert = Some(hex32(&value()?).ok_or_else(usage)?),
            "--seconds" => seconds = value()?.parse().map_err(|_| usage())?,
            "--max-height" => envelope.max_height_m = value()?.parse().map_err(|_| usage())?,
            "--expires-min" => {
                envelope.expires =
                    Duration::from_secs(60 * value()?.parse::<u64>().map_err(|_| usage())?)
            }
            "--refly" => envelope.refly_lines = value()?.parse().map_err(|_| usage())?,
            "--reserve" => envelope.min_reserve = value()?.parse().map_err(|_| usage())?,
            "--no-revise" => envelope.revise = false,
            "--approve" => approve = true,
            other => positional.push(other.to_owned()),
        }
    }
    let digest = |text: Option<&String>| text.and_then(|t| hex_bytes(t)).ok_or_else(usage);
    let action = match positional.first().map(String::as_str) {
        Some("propose") => Action::Propose(positional.get(1).map(PathBuf::from).ok_or_else(usage)?),
        Some("approve") => Action::Approve(digest(positional.get(1))?, envelope),
        Some("start") => Action::Start(digest(positional.get(1))?),
        Some("hold") => Action::Hold,
        Some("resume") => Action::Resume,
        Some("abort") => Action::Abort,
        Some("watch") => Action::Watch,
        Some("run") if approve => Action::Run(
            positional.get(1).map(PathBuf::from).ok_or_else(usage)?,
            envelope,
        ),
        _ => return Err(usage()),
    };
    Ok(Command {
        url: url.ok_or_else(usage)?,
        cert_sha256: cert.ok_or_else(usage)?,
        action,
        seconds,
    })
}

fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok())
        .collect()
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    hex_bytes(text)?.try_into().ok()
}

/// Lower-case hexadecimal.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

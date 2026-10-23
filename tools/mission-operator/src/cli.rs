//! The command line.

use std::path::{Path, PathBuf};

use pilotage_client_session::mission::{ApprovalPolicy, MissionCommand, SurveyFile};
use pilotage_protocol::JoinToken;

/// Why the operator stopped.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OperatorError {
    #[error("usage: {0}")]
    Usage(String),
    #[error("cannot start the async runtime")]
    Runtime(#[source] std::io::Error),
    #[error("cannot read {path:?}: {reason}")]
    File { path: PathBuf, reason: String },
    #[error("the host link failed")]
    Link(#[source] pilotage_client_link::LinkError),
    #[error("the host was not reached or did not answer: {0}")]
    Host(String),
    #[error("cannot read standard input")]
    Stdin(#[source] std::io::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    /// Send one command and print its receipt.
    Send(MissionCommand),
    /// Print the lane's status until it ends.
    Watch,
    /// Propose, approve, start and watch.
    Run(SurveyFile),
    /// Read commands from standard input, one JSON object per line, and
    /// print every receipt and status change.
    Agent,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Command {
    pub url: String,
    pub cert_sha256: [u8; 32],
    /// The secret the host admits this client with: its join secret, or
    /// its approver secret to approve.
    pub token: Option<JoinToken>,
    /// The name the host knows this client by.
    pub name: String,
    pub action: Action,
    /// The approval `run` and `approve` grant.
    pub policy: ApprovalPolicy,
    /// How long `watch` and `run` wait at most.
    pub seconds: u64,
}

const USAGE: &str = "mission-operator --host URL --cert HEX [--token-file PATH] [--name NAME] (propose FILE | approve [DIGEST] [policy] | start [DIGEST] | hold | resume | abort | watch | run FILE --approve [policy] | agent) [--seconds N]; policy: --max-height M --expires-min N --refly N --no-revise --reserve F";

pub(crate) fn parse(args: &[String]) -> Result<Command, OperatorError> {
    let usage = || OperatorError::Usage(USAGE.to_owned());
    let mut url = None;
    let mut cert = None;
    let mut token = None;
    let mut name = "mission-operator".to_owned();
    let mut seconds = 3_600;
    let mut policy = ApprovalPolicy::default();
    let mut approve = false;
    let mut positional = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || iter.next().cloned().ok_or_else(usage);
        match arg.as_str() {
            "--host" => url = Some(value()?),
            "--cert" => cert = Some(hex32(&value()?).ok_or_else(usage)?),
            "--token-file" => token = Some(read_token_blocking(Path::new(&value()?))?),
            "--name" => name = value()?,
            "--seconds" => seconds = value()?.parse().map_err(|_| usage())?,
            "--max-height" => policy.max_height_m = Some(value()?.parse().map_err(|_| usage())?),
            "--expires-min" => policy.expires_min = value()?.parse().map_err(|_| usage())?,
            "--refly" => policy.max_refly_lines = value()?.parse().map_err(|_| usage())?,
            "--reserve" => policy.min_reserve_fraction = value()?.parse().map_err(|_| usage())?,
            "--no-revise" => policy.permit_revise = false,
            "--approve" => approve = true,
            other => positional.push(other.to_owned()),
        }
    }
    let file = |index: usize| -> Result<SurveyFile, OperatorError> {
        read_survey_blocking(Path::new(positional.get(index).ok_or_else(usage)?))
    };
    let digest = positional.get(1).cloned();
    let action = match positional.first().map(String::as_str) {
        Some("propose") => Action::Send(MissionCommand::Propose {
            objective: file(1)?,
        }),
        Some("approve") => Action::Send(MissionCommand::Approve { digest, policy }),
        Some("start") => Action::Send(MissionCommand::Start { digest }),
        Some("hold") => Action::Send(MissionCommand::Hold {}),
        Some("resume") => Action::Send(MissionCommand::Resume {}),
        Some("abort") => Action::Send(MissionCommand::Abort {}),
        Some("watch") => Action::Watch,
        Some("agent") => Action::Agent,
        Some("run") if approve => Action::Run(file(1)?),
        _ => return Err(usage()),
    };
    Ok(Command {
        url: url.ok_or_else(usage)?,
        cert_sha256: cert.ok_or_else(usage)?,
        token,
        name,
        action,
        policy,
        seconds,
    })
}

fn read_blocking(path: &Path) -> Result<String, OperatorError> {
    std::fs::read_to_string(path).map_err(|e| OperatorError::File {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })
}

fn read_survey_blocking(path: &Path) -> Result<SurveyFile, OperatorError> {
    SurveyFile::parse(&read_blocking(path)?).map_err(|e| OperatorError::File {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })
}

fn read_token_blocking(path: &Path) -> Result<JoinToken, OperatorError> {
    JoinToken::from_hex(read_blocking(path)?.trim()).map_err(|e| OperatorError::File {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * index..2 * index + 2)?, 16).ok()?;
    }
    Some(out)
}

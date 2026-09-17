//! Command-line options of the evaluation tool.

use std::path::PathBuf;

use crate::error::EvalError;

/// Usage text, printed for `--help` and with every usage error.
pub(crate) const USAGE: &str = "\
agent-eval: score a model adapter against a case suite

USAGE:
  agent-eval --suite <suite.json> --model-cmd <command> [--out <dir>] [--ledger <file.jsonl>]

  --suite      Case suite: envelope, legend, and cases with the expected directive.
  --model-cmd  Shell command that starts a model adapter (JSON lines on stdin and stdout).
  --out        Directory for the case results and the summary. Default: agent-eval-out
  --ledger     Append-only run ledger. Default: agent-eval-ledger.jsonl";

/// Parsed options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    /// Suite path.
    pub suite: PathBuf,
    /// Shell command that starts the model adapter.
    pub model_cmd: String,
    /// Output directory.
    pub out: PathBuf,
    /// Ledger path.
    pub ledger: PathBuf,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Invocation {
    /// Print the usage text.
    Help,
    /// Run an evaluation.
    Run(Options),
}

/// Parses the arguments that follow the program name.
pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Invocation, EvalError> {
    let usage = |detail: String| EvalError::Usage { detail };
    let (mut suite, mut model_cmd) = (None, None);
    let mut out = PathBuf::from("agent-eval-out");
    let mut ledger = PathBuf::from("agent-eval-ledger.jsonl");
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| usage(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            "--help" | "-h" => return Ok(Invocation::Help),
            "--suite" => suite = Some(PathBuf::from(value()?)),
            "--model-cmd" => model_cmd = Some(value()?),
            "--out" => out = PathBuf::from(value()?),
            "--ledger" => ledger = PathBuf::from(value()?),
            other => return Err(usage(format!("unrecognized argument {other}"))),
        }
    }
    Ok(Invocation::Run(Options {
        suite: suite.ok_or_else(|| usage("--suite is required".to_owned()))?,
        model_cmd: model_cmd.ok_or_else(|| usage("--model-cmd is required".to_owned()))?,
        out,
        ledger,
    }))
}

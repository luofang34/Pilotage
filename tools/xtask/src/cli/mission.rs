//! `cargo xtask mission` arguments.

use std::path::PathBuf;

use super::{required_timeout, required_value};
use crate::error::XtaskError;

/// The fleet file `mission` reads when `--plan` names none.
pub const DEFAULT_PLAN: &str = "sim/missions/princeton-landmarks.json";

/// Options for `cargo xtask mission`.
#[derive(Debug, PartialEq)]
pub struct MissionArgs {
    /// The fleet file, relative to the repository root unless absolute
    /// (`--plan`).
    pub plan: PathBuf,
    /// The decision provider every aircraft judges its pictures with
    /// (`--decision`), as `PILOTAGE_DECISION` takes it.
    pub decision: Option<String>,
    /// Seconds between an aircraft's picture assessments
    /// (`--decision-every`).
    pub decision_every_s: Option<f64>,
    /// The chat endpoint that interprets instructions in words, which the
    /// printed App and agent commands name (`--instruction-model`).
    pub instruction_model: Option<String>,
    /// Leave the fleet file's camera fault out (`--no-fault`).
    pub no_fault: bool,
    /// How long each host may take to listen, s (`--ready-timeout`).
    pub ready_timeout_s: u64,
}

pub(super) fn parse_mission(args: &[String]) -> Result<MissionArgs, XtaskError> {
    let mut mission = MissionArgs {
        plan: PathBuf::from(DEFAULT_PLAN),
        decision: None,
        decision_every_s: None,
        instruction_model: None,
        no_fault: false,
        ready_timeout_s: 60,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--plan" => mission.plan = PathBuf::from(required_value(&mut iter, "--plan")?),
            "--decision" => mission.decision = Some(required_value(&mut iter, "--decision")?),
            "--decision-every" => {
                let raw = required_value(&mut iter, "--decision-every")?;
                let seconds = raw
                    .parse::<f64>()
                    .ok()
                    .filter(|s| s.is_finite() && *s > 0.0)
                    .ok_or_else(|| XtaskError::Usage {
                        message: format!("--decision-every expects seconds above 0, got {raw:?}"),
                    })?;
                mission.decision_every_s = Some(seconds);
            }
            "--no-fault" => mission.no_fault = true,
            "--instruction-model" => {
                mission.instruction_model = Some(required_value(&mut iter, "--instruction-model")?);
            }
            "--ready-timeout" => mission.ready_timeout_s = required_timeout(&mut iter)?,
            other => {
                return Err(XtaskError::Usage {
                    message: format!("unknown mission flag {other:?}"),
                });
            }
        }
    }
    Ok(mission)
}

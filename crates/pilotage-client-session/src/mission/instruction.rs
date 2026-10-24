//! Instructions in a person's words, interpreted into steps for the lanes of the chain
//! of command (ADR-0052, instruction interpretation). The interpreter is a language
//! model behind an OpenAI-style chat endpoint; this module builds its request from the
//! lanes the client follows and the action [`catalog`], and checks its reply against
//! both. It does no I/O: the caller sends the request.
//!
//! An instruction becomes one or more steps, each one catalog action for one lane, in
//! the order the words ask. What a step may be is limited to what the catalog holds and
//! what the lane offers now. An instruction never approves, starts or aborts, a drafted
//! change never applies at once, and a step that names a place waits for the client to
//! find it on its map, so the person who reads the draft decides.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{ApprovalPolicy, MissionCommand, MissionDesk};

mod amount;
pub mod catalog;
mod dispatch;
mod prompt;

use catalog::Action;
pub use dispatch::{Dispatch, Progress};

/// One lane the client can send to, as the interpreter sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainLane {
    /// The lane's name.
    pub lane: String,
    /// The lanes it leads, by name.
    pub members: Vec<String>,
    /// What the lane offers now.
    pub offers: Offers,
}

/// What a lane offers now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offers {
    /// The run may hold.
    pub hold: bool,
    /// The held run may resume.
    pub resume: bool,
    /// A change to the plan may be drafted.
    pub revise: bool,
    /// A new objective may be proposed: the lane is idle or its run ended.
    pub propose: bool,
}

impl ChainLane {
    /// The lane `desk` follows, admitted or not; none before the lane
    /// publishes a status.
    pub fn from_desk(desk: &MissionDesk, admitted: bool) -> Option<Self> {
        let status = desk.status()?;
        let offered = desk.offered(admitted, &ApprovalPolicy::default());
        Some(Self {
            lane: status.lane.clone(),
            members: status.members.iter().map(|m| m.lane.clone()).collect(),
            offers: Offers {
                hold: offered.hold,
                resume: offered.resume,
                revise: offered.revise,
                propose: offered.propose,
            },
        })
    }
}

/// An instruction the interpreter understood: its steps, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    /// The interpreter's restatement, for the person to check.
    pub understood: String,
    /// The steps, in the order the words ask.
    pub steps: Vec<Step>,
}

/// One catalog action for one lane.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// The lane it goes to.
    pub lane: String,
    /// The catalog action's name.
    pub action: String,
    /// What it became.
    pub drafted: Drafted,
}

/// What a step became.
#[derive(Clone, Debug, PartialEq)]
pub enum Drafted {
    /// A command the lane can take as it is.
    Command(MissionCommand),
    /// A new survey of the area the words name, which the client finds on its
    /// map, as a drawn area or a named place, before it proposes it.
    Survey {
        /// The area as the words name it.
        area: String,
    },
}

impl Step {
    /// What sending the step does, in fixed words built from the step itself, so the
    /// person reads what is sent and not only the interpreter's restatement.
    pub fn describe(&self) -> String {
        let lane = &self.lane;
        let command = match &self.drafted {
            Drafted::Survey { area } => {
                return format!(
                    "survey \"{area}\" with {lane}: the area is found on the map first, then {lane} proposes a plan for approval"
                );
            }
            Drafted::Command(command) => command,
        };
        match command {
            MissionCommand::Hold {} => format!("{lane} holds in place and keeps its plan"),
            MissionCommand::Resume {} => format!("{lane} resumes its held plan"),
            MissionCommand::Revise {
                reassign: Some(r), ..
            } => format!(
                "draft for {lane}: move {:.0} % of {}'s unflown lines to {}; it waits for approval",
                r.fraction * 100.0,
                r.from,
                r.to
            ),
            MissionCommand::Revise { finish: true, .. } => format!(
                "draft for {lane}: end after the lines in progress and return home; it waits for approval"
            ),
            other => format!("{lane}: {other:?}"),
        }
    }

    /// The command to send, when the step needs nothing more.
    pub fn command(&self) -> Option<&MissionCommand> {
        match &self.drafted {
            Drafted::Command(command) => Some(command),
            Drafted::Survey { .. } => None,
        }
    }
}

impl Instruction {
    /// Every step's description, in order.
    pub fn describe(&self) -> String {
        let steps: Vec<String> = self.steps.iter().map(Step::describe).collect();
        steps.join("; then ")
    }
}

/// Why a reply is not an instruction the lanes can take.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum InstructionError {
    /// The reply is not the JSON the request asked for.
    #[error("the interpreter's reply is malformed: {0}")]
    Malformed(String),
    /// The interpreter found no command for the instruction.
    #[error("no command fits: {0}")]
    Unable(String),
    /// A step names a lane this client does not follow, so it cannot send it there.
    #[error(
        "{0} is not a lane this client follows; send the instruction from a client that follows it"
    )]
    UnknownLane(String),
    /// The lane does not offer the action now, or the catalog holds no such action.
    #[error("{lane} does not offer {action} now")]
    NotOffered {
        /// The lane.
        lane: String,
        /// The action.
        action: String,
    },
    /// A move names a member the lane does not lead, or one member twice.
    #[error("{lane} cannot move lines from {from:?} to {to:?}")]
    Members {
        /// The lane.
        lane: String,
        /// The member that gives lines.
        from: String,
        /// The member that takes them.
        to: String,
    },
    /// A move states no amount: the instruction's words must name how much
    /// moves, so the interpreter never supplies the number.
    #[error("say how much moves: a percentage, a fraction, or a word such as half or all")]
    NeedsAmount,
    /// The words do not tie the move's fraction to the member that gives the lines,
    /// so the person states it again.
    #[error("say how much of {from}'s lines moves: the words do not state {fraction} for it")]
    UnsureAmount {
        /// The member that gives lines.
        from: String,
        /// The fraction the interpreter gave.
        fraction: f64,
    },
    /// A new survey names no area.
    #[error("say which area to survey: a place on the map or an area drawn on it")]
    NeedsArea,
    /// The lane runs a plan, and only an idle lane takes a new survey now.
    #[error("{0} is running a plan; a new survey waits until it ends, or abort it first")]
    Busy(String),
    /// A move's fraction is outside (0, 1].
    #[error("a move needs a fraction above 0 and at most 1, not {0}")]
    Fraction(f64),
}

/// The chain of command an instruction is interpreted against, with the actions the
/// instruction may become.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    lanes: Vec<ChainLane>,
    actions: Vec<Action>,
}

/// The reply the request's schema asks for.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    understood: String,
    steps: Vec<Map<String, Value>>,
}

impl Chain {
    /// The chain the lanes `lanes` form, each one the client can send to, with the
    /// [`catalog::standard`] actions.
    pub fn new(lanes: Vec<ChainLane>) -> Self {
        Self {
            lanes,
            actions: catalog::standard(),
        }
    }

    /// The lanes, as given.
    pub fn lanes(&self) -> &[ChainLane] {
        &self.lanes
    }

    /// The actions an instruction may become.
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// The chat request that asks the interpreter for the steps of `instruction`, with
    /// `selected` naming what the person has selected.
    pub fn request(&self, instruction: &str, selected: &[String]) -> Value {
        json!({
            "temperature": 0,
            "messages": [
                {"role": "system", "content": self.prompt(selected)},
                {"role": "user", "content": instruction},
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {"name": "mission_instruction", "schema": self.schema()},
            },
        })
    }

    /// The instruction the chat `response` to `instruction` holds, every step checked
    /// against the chain, the catalog and the instruction's own words.
    ///
    /// # Errors
    ///
    /// Returns [`InstructionError`] for the first step the lanes cannot take, or when the
    /// response holds no reply of the requested form.
    pub fn parse(
        &self,
        instruction: &str,
        response: &Value,
    ) -> Result<Instruction, InstructionError> {
        let content = response
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| InstructionError::Malformed("no message content".to_owned()))?;
        let reply: Reply = serde_json::from_str(content)
            .map_err(|e| InstructionError::Malformed(e.to_string()))?;
        if reply.steps.is_empty() || reply.steps.len() > prompt::MAX_STEPS {
            return Err(InstructionError::Malformed(format!(
                "{} steps",
                reply.steps.len()
            )));
        }
        let steps = reply
            .steps
            .iter()
            .map(|step| self.step(step, instruction, &reply.understood))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Instruction {
            understood: reply.understood,
            steps,
        })
    }

    fn step(
        &self,
        args: &Map<String, Value>,
        words: &str,
        understood: &str,
    ) -> Result<Step, InstructionError> {
        let field = |name: &str| {
            args.get(name)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let (name, lane_name) = (field("action"), field("lane"));
        if name == "unable" {
            return Err(InstructionError::Unable(understood.to_owned()));
        }
        let lane = self
            .lanes
            .iter()
            .find(|l| l.lane == lane_name)
            .ok_or_else(|| InstructionError::UnknownLane(lane_name.clone()))?;
        let action = self
            .actions
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| InstructionError::NotOffered {
                lane: lane_name.clone(),
                action: name.clone(),
            })?;
        Ok(Step {
            lane: lane_name,
            action: name,
            drafted: catalog::build(action, lane, args, words)?,
        })
    }
}

#[cfg(test)]
mod tests;

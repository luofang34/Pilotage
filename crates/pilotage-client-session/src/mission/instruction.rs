//! Instructions in a person's words, interpreted into one [`MissionCommand`]
//! for one lane of the chain of command (ADR-0052, instruction
//! interpretation). The interpreter is a language model behind an
//! OpenAI-style chat endpoint; this module builds its request from the
//! lanes the client follows and checks its reply against them. It does no
//! I/O: the caller sends the request.
//!
//! What an instruction may become is limited to what the lanes offer and
//! what changes the plan only through review: hold, resume, a drafted move
//! of unflown lines, and a drafted finish. An instruction never approves,
//! starts or aborts, and a drafted change never applies at once, so the
//! person who reads the draft decides.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{ApprovalPolicy, MissionCommand, MissionDesk, Reassign};

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

/// The instruction actions a lane offers now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offers {
    /// The run may hold.
    pub hold: bool,
    /// The held run may resume.
    pub resume: bool,
    /// A change to the plan may be drafted.
    pub revise: bool,
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
            },
        })
    }
}

/// An instruction the interpreter understood.
#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    /// The lane the command goes to.
    pub lane: String,
    /// The interpreter's restatement, for the person to check.
    pub understood: String,
    /// The command.
    pub command: MissionCommand,
}

impl Instruction {
    /// What sending the command does, in fixed words built from the command
    /// itself, so the person reads what is sent and not only the
    /// interpreter's restatement.
    pub fn describe(&self) -> String {
        let lane = &self.lane;
        match &self.command {
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
    /// The reply names a lane this client does not follow, so it cannot
    /// send the command there.
    #[error(
        "{0} is not a lane this client follows; send the instruction from a client that follows it"
    )]
    UnknownLane(String),
    /// The lane does not offer the action now.
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
    #[error("say how much moves: a number, a percentage, or a word such as half or all")]
    NeedsAmount,
    /// A move's fraction is outside (0, 1].
    #[error("a move needs a fraction above 0 and at most 1, not {0}")]
    Fraction(f64),
}

/// The chain of command an instruction is interpreted against.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    lanes: Vec<ChainLane>,
}

/// The reply the request's schema asks for.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    understood: String,
    action: String,
    lane: String,
    from: String,
    to: String,
    fraction: f64,
}

impl Chain {
    /// The chain the lanes `lanes` form; each is one the client can send to.
    pub fn new(lanes: Vec<ChainLane>) -> Self {
        Self { lanes }
    }

    /// The lanes, as given.
    pub fn lanes(&self) -> &[ChainLane] {
        &self.lanes
    }

    /// The chat request that asks the interpreter for one command for
    /// `instruction`, with `selected` naming what the person has selected.
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

    /// The instruction the chat `response` to `instruction` holds, checked
    /// against the chain and against the instruction's own words.
    ///
    /// # Errors
    ///
    /// Returns [`InstructionError`] when the response holds no reply of the
    /// requested form, the reply names something the chain cannot take, or
    /// it states an amount the instruction does not.
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
        if reply.action == "move_lines" && !states_an_amount(instruction) {
            return Err(InstructionError::NeedsAmount);
        }
        self.check(reply)
    }

    fn check(&self, reply: Reply) -> Result<Instruction, InstructionError> {
        if reply.action == "unable" {
            return Err(InstructionError::Unable(reply.understood));
        }
        let lane = self
            .lanes
            .iter()
            .find(|l| l.lane == reply.lane)
            .ok_or_else(|| InstructionError::UnknownLane(reply.lane.clone()))?;
        let not_offered = || InstructionError::NotOffered {
            lane: lane.lane.clone(),
            action: reply.action.clone(),
        };
        let command = match reply.action.as_str() {
            "hold" if lane.offers.hold => MissionCommand::Hold {},
            "resume" if lane.offers.resume => MissionCommand::Resume {},
            "finish" if lane.offers.revise => revise(None, true),
            "move_lines" if lane.offers.revise => {
                let led = |m: &str| lane.members.iter().any(|x| x == m);
                if reply.from == reply.to || !led(&reply.from) || !led(&reply.to) {
                    return Err(InstructionError::Members {
                        lane: lane.lane.clone(),
                        from: reply.from,
                        to: reply.to,
                    });
                }
                if !(reply.fraction > 0.0 && reply.fraction <= 1.0) {
                    return Err(InstructionError::Fraction(reply.fraction));
                }
                revise(
                    Some(Reassign {
                        from: reply.from,
                        to: reply.to,
                        fraction: reply.fraction,
                    }),
                    false,
                )
            }
            _ => return Err(not_offered()),
        };
        Ok(Instruction {
            lane: lane.lane.clone(),
            understood: reply.understood,
            command,
        })
    }

    /// The actions any lane offers now, always with `unable`.
    fn actions(&self) -> Vec<&'static str> {
        let any = |f: fn(&Offers) -> bool| self.lanes.iter().any(|l| f(&l.offers));
        let mut actions = Vec::new();
        if any(|o| o.hold) {
            actions.push("hold");
        }
        if any(|o| o.resume) {
            actions.push("resume");
        }
        if any(|o| o.revise) {
            actions.extend(["finish", "move_lines"]);
        }
        actions.push("unable");
        actions
    }

    fn schema(&self) -> Value {
        // Every lane the chain names may receive the command in the reply, so
        // an instruction about a lane this client does not follow is refused
        // rather than sent to the lead above it.
        let mut lanes: Vec<&str> = self
            .lanes
            .iter()
            .flat_map(|l| {
                std::iter::once(l.lane.as_str()).chain(l.members.iter().map(String::as_str))
            })
            .collect();
        lanes.sort_unstable();
        lanes.dedup();
        let mut names = lanes.clone();
        names.push("");
        json!({
            "type": "object",
            "properties": {
                "understood": {"type": "string"},
                "action": {"type": "string", "enum": self.actions()},
                "lane": {"type": "string", "enum": lanes},
                "from": {"type": "string", "enum": names},
                "to": {"type": "string", "enum": names},
                "fraction": {"type": "number"},
            },
            "required": ["understood", "action", "lane", "from", "to", "fraction"],
            "additionalProperties": false,
        })
    }

    fn prompt(&self, selected: &[String]) -> String {
        let mut text = String::from(
            "You turn an operator's instruction into one command for one lane of a chain of command of uncrewed aircraft.\nChain:\n",
        );
        for lane in &self.lanes {
            let leads = if lane.members.is_empty() {
                String::new()
            } else {
                format!(" leads {}", lane.members.join(", "))
            };
            text.push_str(&format!("- {}{leads}\n", lane.lane));
        }
        text.push_str("Actions:\n");
        for action in self.actions() {
            text.push_str(match action {
                "hold" => "- hold: stop in place and keep the plan.\n",
                "resume" => "- resume: continue a held plan.\n",
                "finish" => "- finish: end after the lines in progress and return home.\n",
                "move_lines" => "- move_lines: move a fraction (0..1] of the unflown lines of member \"from\" to member \"to\"; both are members \"lane\" leads.\n",
                _ => "- unable: no action fits, for example a new kind of mission; say why in \"understood\".\n",
            });
        }
        text.push_str(
            "\"lane\" receives the command: the lowest lane that leads everything the instruction names. Use \"\" and 0 for fields an action does not use. Restate the command in \"understood\".\n",
        );
        if !selected.is_empty() {
            text.push_str(&format!(
                "The operator has selected: {}. Words such as \"these\" or \"them\" mean the selection.\n",
                selected.join(", ")
            ));
        }
        text
    }
}

/// Whether `instruction` states an amount: a digit, or a word for one.
fn states_an_amount(instruction: &str) -> bool {
    const WORDS: [&str; 8] = [
        "all",
        "every",
        "half",
        "third",
        "quarter",
        "rest",
        "remaining",
        "whole",
    ];
    let lower = instruction.to_lowercase();
    lower.chars().any(|c| c.is_ascii_digit())
        || lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| WORDS.iter().any(|w| word.starts_with(w)))
}

fn revise(reassign: Option<Reassign>, finish: bool) -> MissionCommand {
    MissionCommand::Revise {
        objective: None,
        reassign,
        finish,
        // A person reads every draft before it applies.
        within_envelope: false,
    }
}

#[cfg(test)]
mod tests;

//! The actions an instruction may become, as descriptors: a name, a description in
//! words, typed parameters, the lane state that offers the action, and what sending it
//! does. The interpreter's request is generated from the descriptors, so an action is
//! added by adding its descriptor and its builder, and the descriptors serialize in the
//! form lanes may later publish themselves, or a tool server may offer to any agent.
//!
//! Approve, start and abort are never actions: a person approves, starts and aborts
//! through their own controls.

use serde::Serialize;
use serde_json::{Map, Value};

use super::{ChainLane, Drafted, InstructionError, Offers, amount};
use crate::mission::{MissionCommand, Reassign};

/// One action an instruction may become.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Action {
    /// The name the interpreter calls it by.
    pub name: &'static str,
    /// What it does, in words the interpreter reads.
    pub description: &'static str,
    /// Its parameters.
    pub params: Vec<Param>,
    /// The lane state that offers it.
    pub offered: Precondition,
    /// What sending it does.
    pub effect: Effect,
}

/// One parameter of an action.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Param {
    /// The parameter's name in a step of the reply.
    pub name: &'static str,
    /// What it holds.
    pub kind: ParamKind,
    /// What it means, in words the interpreter reads.
    pub description: &'static str,
}

/// What a parameter holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// A member lane of the lane the step goes to.
    Member,
    /// A share in (0, 1] that the person's words must state.
    Amount,
    /// A place or a drawn area as the words name it, which the client finds on its map;
    /// the interpreter never gives coordinates.
    Area,
}

/// The lane state that offers an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Precondition {
    /// A running plan.
    Running,
    /// A held plan.
    Held,
    /// A plan a change may be drafted for.
    Revisable,
    /// An idle lane, or one whose run ended.
    Idle,
}

impl Precondition {
    /// Whether a lane offering `offers` meets the precondition.
    pub fn met(self, offers: &Offers) -> bool {
        match self {
            Self::Running => offers.hold,
            Self::Held => offers.resume,
            Self::Revisable => offers.revise,
            Self::Idle => offers.propose,
        }
    }
}

/// What sending an action does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// The lane acts at once, as with the panel's button.
    Immediate,
    /// The lane drafts it, and it waits for an approval.
    Draft,
}

const FROM: Param = Param {
    name: "from",
    kind: ParamKind::Member,
    description: "the member that gives lines",
};
const TO: Param = Param {
    name: "to",
    kind: ParamKind::Member,
    description: "the member that takes them",
};
const FRACTION: Param = Param {
    name: "fraction",
    kind: ParamKind::Amount,
    description: "the share of the unflown lines that moves, as the words state it",
};
const AREA: Param = Param {
    name: "area",
    kind: ParamKind::Area,
    description: "the area as the words name it: a road, a park, a place, or an area drawn on the map",
};

/// The actions every lane of the survey protocol offers.
pub fn standard() -> Vec<Action> {
    vec![
        Action {
            name: "hold",
            description: "stop in place and keep the plan",
            params: Vec::new(),
            offered: Precondition::Running,
            effect: Effect::Immediate,
        },
        Action {
            name: "resume",
            description: "continue a held plan",
            params: Vec::new(),
            offered: Precondition::Held,
            effect: Effect::Immediate,
        },
        Action {
            name: "finish",
            description: "end after the lines in progress and return home",
            params: Vec::new(),
            offered: Precondition::Revisable,
            effect: Effect::Draft,
        },
        Action {
            name: "move_lines",
            description: "move a share of the unflown lines of one member to another member of the lane",
            params: vec![FROM, TO, FRACTION],
            offered: Precondition::Revisable,
            effect: Effect::Draft,
        },
        Action {
            name: "survey_area",
            description: "plan a new survey of an area the words name",
            params: vec![AREA],
            offered: Precondition::Idle,
            effect: Effect::Draft,
        },
    ]
}

/// What `action` with `args` becomes for `lane`, checked against the lane and against
/// the person's `words`.
pub(super) fn build(
    action: &Action,
    lane: &ChainLane,
    args: &Map<String, Value>,
    words: &str,
) -> Result<Drafted, InstructionError> {
    let text = |name: &str| {
        args.get(name)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    if action.params.iter().any(|p| p.kind == ParamKind::Area) && text("area").is_empty() {
        return Err(InstructionError::NeedsArea);
    }
    if !action.offered.met(&lane.offers) {
        return Err(match action.offered {
            Precondition::Idle => InstructionError::Busy(lane.lane.clone()),
            _ => InstructionError::NotOffered {
                lane: lane.lane.clone(),
                action: action.name.to_owned(),
            },
        });
    }
    let command = match action.name {
        "hold" => MissionCommand::Hold {},
        "resume" => MissionCommand::Resume {},
        "finish" => revise(None, true),
        "move_lines" => {
            let (from, to) = (text("from"), text("to"));
            let led = |m: &str| lane.members.iter().any(|x| x == m);
            if from == to || !led(&from) || !led(&to) {
                return Err(InstructionError::Members {
                    lane: lane.lane.clone(),
                    from,
                    to,
                });
            }
            let fraction = args.get("fraction").and_then(Value::as_f64).unwrap_or(0.0);
            if !(fraction > 0.0 && fraction <= 1.0) {
                return Err(InstructionError::Fraction(fraction));
            }
            amount::ground(words, &from, fraction)?;
            revise(Some(Reassign { from, to, fraction }), false)
        }
        "survey_area" => return Ok(Drafted::Survey { area: text("area") }),
        other => {
            return Err(InstructionError::NotOffered {
                lane: lane.lane.clone(),
                action: other.to_owned(),
            });
        }
    };
    Ok(Drafted::Command(command))
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

//! The commands a client sends a mission lane, in the words every client
//! uses. Their JSON form is one object per line, tagged by `command`:
//!
//! ```json
//! {"command": "propose", "objective": {"title": "...", "area": [...], "launch": [...]}}
//! {"command": "approve", "policy": {"max_refly_lines": 2}}
//! {"command": "start"}
//! {"command": "hold"}
//! ```
//!
//! `approve` and `start` name the proposal by its hexadecimal digest, or
//! take the one the desk holds when the digest is absent. A field the
//! command does not take is refused, not ignored.

use pilotage_protocol::wire;
use serde::{Deserialize, Serialize};

use super::{ApprovalPolicy, SurveyFile};

/// One command to a mission lane.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum MissionCommand {
    /// Compile `objective` into a proposal.
    Propose {
        /// The survey to fly.
        objective: SurveyFile,
    },
    /// Approve a proposal under `policy`.
    Approve {
        /// The proposal's digest; absent takes the desk's proposal.
        #[serde(default)]
        digest: Option<String>,
        /// What the approval permits.
        #[serde(default)]
        policy: ApprovalPolicy,
    },
    /// Start an approved proposal.
    Start {
        /// The proposal's digest; absent takes the one the lane names.
        #[serde(default)]
        digest: Option<String>,
    },
    /// Stop in place and keep the plan.
    Hold {},
    /// Continue a held plan.
    Resume {},
    /// End the plan; each vehicle returns.
    Abort {},
}

/// What an intent asks of the lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    /// Compile an objective.
    Propose,
    /// Approve a proposal.
    Approve,
    /// Start an approved proposal.
    Start,
    /// Stop in place.
    Hold,
    /// Continue a held plan.
    Resume,
    /// End the plan.
    Abort,
    /// Skip a task.
    Skip,
}

impl IntentKind {
    /// The kind of `action`.
    #[must_use]
    pub fn of(action: &wire::mission_intent::Action) -> Self {
        use wire::mission_intent::Action;
        match action {
            Action::Propose(_) => Self::Propose,
            Action::Approve(_) => Self::Approve,
            Action::Start(_) => Self::Start,
            Action::Hold(_) => Self::Hold,
            Action::Resume(_) => Self::Resume,
            Action::Abort(_) => Self::Abort,
            Action::Skip(_) => Self::Skip,
        }
    }

    /// The operator's word for the intent.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Propose => "Propose",
            Self::Approve => "Approve",
            Self::Start => "Start",
            Self::Hold => "Hold",
            Self::Resume => "Resume",
            Self::Abort => "Abort",
            Self::Skip => "Skip",
        }
    }
}

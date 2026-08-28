//! Typed mission directives and correlated receipts.

use serde::{Deserialize, Serialize};

use crate::{FlightAction, MissionAction, TrialAction};

/// A nonzero identifier for one directive attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionId(pub(crate) u32);

impl ActionId {
    /// Gets the identifier value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The reason that the engine emitted a directive.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "purpose", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectivePurpose {
    /// Execute the main action of a phase.
    PhaseAction {},
    /// Execute one declared cleanup action.
    Cleanup {
        /// The zero-based index in the phase cleanup list.
        cleanup_index: usize,
    },
}

/// Correlation and phase data for one directive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectiveContext {
    /// The identifier that a receipt must return.
    pub action_id: ActionId,
    /// The zero-based phase index.
    pub phase_index: usize,
    /// The stable phase identifier.
    pub phase_id: String,
    /// The one-based attempt number for this action.
    pub attempt: u32,
    /// The reason for the directive.
    pub purpose: DirectivePurpose,
}

/// An operational flight directive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightDirective {
    /// The directive context.
    pub context: DirectiveContext,
    /// The flight action for a host handler.
    pub action: FlightAction,
}

/// A simulator-only trial directive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialDirective {
    /// The directive context.
    pub context: DirectiveContext,
    /// The trial action for a host handler.
    pub action: TrialAction,
}

/// A typed directive on one transport lane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "lane", content = "directive", rename_all = "snake_case")]
pub enum MissionDirective {
    /// A directive for the operational command path.
    Flight(FlightDirective),
    /// A directive for the simulator-only command path.
    Trial(TrialDirective),
}

/// The host result for one directive attempt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReceiptResult {
    /// The host completed the directive.
    Succeeded {},
    /// The host asks the engine to retry the directive.
    Retryable {
        /// The host detail for evidence.
        detail: String,
    },
    /// The host does not support or admit the directive.
    Refused {
        /// The host refusal detail.
        detail: String,
    },
    /// The host attempted and failed the directive.
    Failed {
        /// The host failure detail.
        detail: String,
    },
}

/// A receipt correlated to one directive attempt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectiveReceipt {
    /// The identifier from the directive.
    pub action_id: ActionId,
    /// The host result.
    pub result: ReceiptResult,
}

impl MissionDirective {
    pub(crate) fn new(context: DirectiveContext, action: MissionAction) -> Self {
        match action {
            MissionAction::Flight(action) => Self::Flight(FlightDirective { context, action }),
            MissionAction::Trial(action) => Self::Trial(TrialDirective { context, action }),
        }
    }

    /// Gets the directive context.
    #[must_use]
    pub const fn context(&self) -> &DirectiveContext {
        match self {
            Self::Flight(directive) => &directive.context,
            Self::Trial(directive) => &directive.context,
        }
    }

    pub(crate) const fn action_name(&self) -> &'static str {
        match self {
            Self::Flight(directive) => directive.action.name(),
            Self::Trial(directive) => directive.action.name(),
        }
    }

    pub(crate) fn action(&self) -> MissionAction {
        match self {
            Self::Flight(directive) => MissionAction::Flight(directive.action.clone()),
            Self::Trial(directive) => MissionAction::Trial(directive.action.clone()),
        }
    }
}

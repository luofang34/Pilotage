//! Engine states, evidence events, and typed terminal results.

use serde::{Deserialize, Serialize};

use crate::{ActionId, DirectiveContext, MissionDirective, ReceiptResult};

/// The deadline class that stopped a mission.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "class", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeadlineClass {
    /// The active phase used all of its simulator-time duration.
    PhaseSimulatorTime {
        /// The phase index.
        phase_index: usize,
        /// The stable phase identifier.
        phase_id: String,
        /// The configured phase duration.
        limit_ns: u64,
        /// The observed duration when the engine stopped the phase.
        elapsed_ns: u64,
    },
    /// The mission reached its absolute caller wall deadline.
    MissionWall {
        /// The absolute deadline value.
        deadline_ns: u64,
        /// The caller wall clock that reached the deadline.
        observed_ns: u64,
    },
}

/// The reason for an aborted terminal result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case", deny_unknown_fields)]
pub enum AbortCause {
    /// One declared abort condition matched.
    Condition {
        /// The zero-based condition index.
        condition_index: usize,
    },
    /// A host attempted the action and reported a failure.
    ActionFailed {
        /// The host failure detail.
        detail: String,
    },
    /// Retryable receipts used all permitted retries.
    RetryLimitExceeded {
        /// The last host retry detail.
        detail: String,
        /// The configured retry limit.
        retry_limit: u16,
    },
}

/// The failure class for one cleanup step.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "failure", rename_all = "snake_case", deny_unknown_fields)]
pub enum CleanupFailureKind {
    /// The host refused the cleanup action.
    Refused {
        /// The host refusal detail.
        detail: String,
    },
    /// The host attempted and failed the cleanup action.
    Failed {
        /// The host failure detail.
        detail: String,
    },
    /// Retryable receipts used all permitted retries.
    RetryLimitExceeded {
        /// The last host retry detail.
        detail: String,
        /// The configured retry limit.
        retry_limit: u16,
    },
    /// The cleanup action did not produce a receipt in time.
    ReceiptTimeout {
        /// The timed-out directive identifier.
        action_id: ActionId,
    },
}

/// One failed cleanup step in an unsuccessful mission result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupFailure {
    /// The phase index that owns the cleanup action.
    pub phase_index: usize,
    /// The stable phase identifier.
    pub phase_id: String,
    /// The zero-based index in the phase cleanup list.
    pub cleanup_index: usize,
    /// The stable cleanup action name.
    pub action: String,
    /// The cleanup failure class.
    pub failure: CleanupFailureKind,
}

/// The one typed terminal result for a mission run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "terminal", rename_all = "snake_case", deny_unknown_fields)]
pub enum MissionTerminal {
    /// All mission phases completed.
    Complete {
        /// The number of completed phases.
        completed_phases: usize,
    },
    /// A mission abort condition or action failure stopped the run.
    Aborted {
        /// The phase index at the stop.
        phase_index: usize,
        /// The stable phase identifier.
        phase_id: String,
        /// The stable phase action name.
        action: String,
        /// The abort cause.
        cause: AbortCause,
        /// All cleanup failures.
        cleanup_failures: Vec<CleanupFailure>,
    },
    /// The host refused a phase action.
    Refused {
        /// The refusing phase index.
        phase_index: usize,
        /// The stable phase identifier.
        phase_id: String,
        /// The stable refused action name.
        action: String,
        /// The host refusal detail.
        detail: String,
        /// All cleanup failures.
        cleanup_failures: Vec<CleanupFailure>,
    },
    /// A simulator-time or wall deadline stopped the run.
    DeadlineExceeded {
        /// The distinct deadline class and values.
        deadline: DeadlineClass,
        /// All cleanup failures.
        cleanup_failures: Vec<CleanupFailure>,
    },
    /// A phase action did not produce a receipt in time.
    ReceiptTimeout {
        /// The phase index.
        phase_index: usize,
        /// The stable phase identifier.
        phase_id: String,
        /// The stable action name.
        action: String,
        /// The timed-out directive identifier.
        action_id: ActionId,
        /// All cleanup failures.
        cleanup_failures: Vec<CleanupFailure>,
    },
}

/// The progress stage of the active mission phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub enum PhaseStage {
    /// The engine waits for all entry conditions.
    WaitingForEntry {},
    /// The engine waits for the current action receipt.
    WaitingForReceipt {
        /// The outstanding directive identifier.
        action_id: ActionId,
    },
    /// The engine waits for all completion conditions.
    WaitingForCompletion {},
}

/// A public snapshot of mission engine state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngineState {
    /// The engine is executing one mission phase.
    Running {
        /// The active phase index.
        phase_index: usize,
        /// The stable phase identifier.
        phase_id: String,
        /// The active phase progress stage.
        stage: PhaseStage,
    },
    /// The engine is attempting all remaining cleanup steps.
    CleaningUp {
        /// The number of steps not yet completed or failed.
        remaining_steps: usize,
        /// The outstanding cleanup directive, when present.
        outstanding_action_id: Option<ActionId>,
    },
    /// The mission has one final result.
    Terminal {
        /// The typed terminal result.
        result: MissionTerminal,
    },
}

/// A typed evidence event from one engine tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngineEvent {
    /// All entry conditions permitted a phase to start.
    PhaseEntered {
        /// The phase index.
        phase_index: usize,
        /// The stable phase identifier.
        phase_id: String,
        /// The caller simulator clock.
        simulator_time_ns: u64,
    },
    /// All completion conditions completed a phase.
    PhaseCompleted {
        /// The phase index.
        phase_index: usize,
        /// The stable phase identifier.
        phase_id: String,
        /// The caller simulator clock.
        simulator_time_ns: u64,
    },
    /// One abort condition matched.
    AbortConditionMatched {
        /// The phase index.
        phase_index: usize,
        /// The stable phase identifier.
        phase_id: String,
        /// The zero-based abort condition index.
        condition_index: usize,
    },
    /// The engine emitted one typed directive.
    DirectiveEmitted {
        /// The complete emitted directive.
        directive: MissionDirective,
    },
    /// The engine accepted one exactly correlated receipt.
    ReceiptAccepted {
        /// The directive context.
        context: DirectiveContext,
        /// The accepted host result.
        result: ReceiptResult,
    },
    /// A retryable receipt caused another directive attempt.
    DirectiveRetried {
        /// The resolved directive identifier.
        previous_action_id: ActionId,
        /// The new retry directive.
        directive: MissionDirective,
    },
    /// A host refusal stopped the mission action.
    Refusal {
        /// The refused directive context.
        context: DirectiveContext,
        /// The stable refused action name.
        action: String,
        /// The host refusal detail.
        detail: String,
    },
    /// One of the two deadline classes stopped the mission.
    DeadlineExceeded {
        /// The deadline class and values.
        deadline: DeadlineClass,
    },
    /// A phase directive did not produce a receipt in time.
    ReceiptTimedOut {
        /// The timed-out directive context.
        context: DirectiveContext,
        /// The stable action name.
        action: String,
    },
    /// One cleanup step failed and cleanup continued.
    CleanupFailed {
        /// The typed cleanup failure.
        failure: CleanupFailure,
    },
    /// The engine produced its one terminal result.
    Terminal {
        /// The complete terminal result.
        result: MissionTerminal,
    },
}

/// Directives, evidence, and state returned by one pure tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickOutput {
    /// Directives for host execution.
    pub directives: Vec<MissionDirective>,
    /// Typed evidence events in deterministic order.
    pub events: Vec<EngineEvent>,
    /// The engine state after the tick.
    pub state: EngineState,
}

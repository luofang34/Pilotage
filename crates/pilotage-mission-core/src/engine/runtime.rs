//! Shared runtime state for sequencing and cleanup.

use crate::{MissionAction, MissionDocument};

use super::{
    AbortCause, ActionId, CleanupFailure, DirectiveContext, DirectivePurpose, EngineEvent,
    EngineState, MissionDirective, MissionTerminal, PhaseStage, TickInput,
};

#[derive(Clone, Debug)]
pub(super) enum RunState {
    Running(RunningState),
    CleaningUp(Box<CleanupState>),
    Terminal(MissionTerminal),
}

#[derive(Clone, Debug)]
pub(super) struct RunningState {
    pub(super) phase_index: usize,
    pub(super) phase_started_at_ns: u64,
    pub(super) progress: PhaseProgress,
}

#[derive(Clone, Debug)]
pub(super) enum PhaseProgress {
    Entry,
    Receipt(Box<PendingDirective>),
    Completion,
}

#[derive(Clone, Debug)]
pub(super) struct PendingDirective {
    pub(super) directive: MissionDirective,
    pub(super) emitted_at_wall_ns: u64,
    pub(super) retries_used: u16,
}

#[derive(Clone, Debug)]
pub(super) struct CleanupStep {
    pub(super) phase_index: usize,
    pub(super) phase_id: String,
    pub(super) cleanup_index: usize,
    pub(super) action: MissionAction,
}

#[derive(Clone, Debug)]
pub(super) struct CleanupState {
    pub(super) cause: PendingTerminal,
    pub(super) steps: Vec<CleanupStep>,
    pub(super) cursor: usize,
    pub(super) pending: Option<PendingDirective>,
    pub(super) failures: Vec<CleanupFailure>,
}

#[derive(Clone, Debug)]
pub(super) enum PendingTerminal {
    Aborted {
        phase_index: usize,
        phase_id: String,
        action: String,
        cause: AbortCause,
    },
    Refused {
        phase_index: usize,
        phase_id: String,
        action: String,
        detail: String,
    },
    DeadlineExceeded(super::DeadlineClass),
    ReceiptTimeout {
        phase_index: usize,
        phase_id: String,
        action: String,
        action_id: ActionId,
    },
}

#[derive(Default)]
pub(super) struct TickCollector {
    pub(super) directives: Vec<MissionDirective>,
    pub(super) events: Vec<EngineEvent>,
}

pub(super) struct TickContext<'a> {
    pub(super) document: &'a MissionDocument,
    pub(super) entered_phases: &'a mut Vec<usize>,
    pub(super) last_action_id: &'a mut u32,
    pub(super) input: &'a TickInput,
    pub(super) collector: &'a mut TickCollector,
}

pub(super) fn new_pending(
    context: &mut TickContext<'_>,
    phase_index: usize,
    purpose: DirectivePurpose,
    action: MissionAction,
    retries_used: u16,
) -> PendingDirective {
    let action_id = next_action_id(context.last_action_id);
    let directive_context = DirectiveContext {
        action_id,
        phase_index,
        phase_id: context.document.phases[phase_index].id.clone(),
        attempt: u32::from(retries_used).wrapping_add(1),
        purpose,
    };
    PendingDirective {
        directive: MissionDirective::new(directive_context, action),
        emitted_at_wall_ns: context.input.wall_time_ns,
        retries_used,
    }
}

pub(super) fn emit(directive: &MissionDirective, collector: &mut TickCollector) {
    collector.directives.push(directive.clone());
    collector.events.push(EngineEvent::DirectiveEmitted {
        directive: directive.clone(),
    });
}

pub(super) fn terminal_state(result: MissionTerminal, collector: &mut TickCollector) -> RunState {
    collector.events.push(EngineEvent::Terminal {
        result: result.clone(),
    });
    RunState::Terminal(result)
}

pub(super) fn running_snapshot(document: &MissionDocument, state: &RunningState) -> EngineState {
    let stage = match &state.progress {
        PhaseProgress::Entry => PhaseStage::WaitingForEntry {},
        PhaseProgress::Receipt(pending) => PhaseStage::WaitingForReceipt {
            action_id: pending.directive.context().action_id,
        },
        PhaseProgress::Completion => PhaseStage::WaitingForCompletion {},
    };
    EngineState::Running {
        phase_index: state.phase_index,
        phase_id: document.phases[state.phase_index].id.clone(),
        stage,
    }
}

pub(super) fn cleanup_snapshot(state: &CleanupState) -> EngineState {
    EngineState::CleaningUp {
        remaining_steps: state.steps.len().saturating_sub(state.cursor),
        outstanding_action_id: state
            .pending
            .as_ref()
            .map(|pending| pending.directive.context().action_id),
    }
}

fn next_action_id(last_action_id: &mut u32) -> ActionId {
    *last_action_id = last_action_id.wrapping_add(1);
    if *last_action_id == 0 {
        *last_action_id = last_action_id.wrapping_add(1);
    }
    ActionId(*last_action_id)
}

impl PendingTerminal {
    pub(super) fn finish(self, cleanup_failures: Vec<CleanupFailure>) -> MissionTerminal {
        match self {
            Self::Aborted {
                phase_index,
                phase_id,
                action,
                cause,
            } => MissionTerminal::Aborted {
                phase_index,
                phase_id,
                action,
                cause,
                cleanup_failures,
            },
            Self::Refused {
                phase_index,
                phase_id,
                action,
                detail,
            } => MissionTerminal::Refused {
                phase_index,
                phase_id,
                action,
                detail,
                cleanup_failures,
            },
            Self::DeadlineExceeded(deadline) => MissionTerminal::DeadlineExceeded {
                deadline,
                cleanup_failures,
            },
            Self::ReceiptTimeout {
                phase_index,
                phase_id,
                action,
                action_id,
            } => MissionTerminal::ReceiptTimeout {
                phase_index,
                phase_id,
                action,
                action_id,
                cleanup_failures,
            },
        }
    }
}

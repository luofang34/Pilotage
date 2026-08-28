//! Reverse-phase attempt-all cleanup.

use super::runtime::{
    CleanupState, CleanupStep, PendingTerminal, RunState, TickContext, emit, new_pending,
    terminal_state,
};
use super::{CleanupFailure, CleanupFailureKind, DirectivePurpose, EngineEvent, ReceiptResult};

pub(super) fn start_cleanup(cause: PendingTerminal, context: &mut TickContext<'_>) -> RunState {
    let steps = cleanup_steps(context);
    let mut state = CleanupState {
        cause,
        steps,
        cursor: 0,
        pending: None,
        failures: Vec::new(),
    };
    issue_or_finish(&mut state, context)
}

pub(super) fn tick_cleanup(
    state: &mut CleanupState,
    mut context: TickContext<'_>,
) -> Option<RunState> {
    let Some(pending) = &state.pending else {
        return Some(issue_or_finish(state, &mut context));
    };
    if let Some(receipt) = context.input.receipts.first() {
        let result = receipt.result.clone();
        context.collector.events.push(EngineEvent::ReceiptAccepted {
            context: pending.directive.context().clone(),
            result: result.clone(),
        });
        return process_receipt(state, result, &mut context);
    }
    let elapsed = context
        .input
        .wall_time_ns
        .saturating_sub(pending.emitted_at_wall_ns);
    if elapsed < context.document.execution_policy.receipt_timeout_ns {
        return None;
    }
    let action_id = pending.directive.context().action_id;
    record_failure(
        state,
        CleanupFailureKind::ReceiptTimeout { action_id },
        context.collector,
    );
    Some(advance(state, &mut context))
}

fn cleanup_steps(context: &TickContext<'_>) -> Vec<CleanupStep> {
    let mut steps = Vec::new();
    for phase_index in context.entered_phases.iter().rev().copied() {
        let phase = &context.document.phases[phase_index];
        for (cleanup_index, action) in phase.cleanup_actions.iter().enumerate() {
            steps.push(CleanupStep {
                phase_index,
                phase_id: phase.id.clone(),
                cleanup_index,
                action: action.clone(),
            });
        }
    }
    steps
}

fn process_receipt(
    state: &mut CleanupState,
    result: ReceiptResult,
    context: &mut TickContext<'_>,
) -> Option<RunState> {
    match result {
        ReceiptResult::Succeeded {} => Some(advance(state, context)),
        ReceiptResult::Retryable { detail } => retry(state, detail, context),
        ReceiptResult::Refused { detail } => {
            record_failure(
                state,
                CleanupFailureKind::Refused { detail },
                context.collector,
            );
            Some(advance(state, context))
        }
        ReceiptResult::Failed { detail } => {
            record_failure(
                state,
                CleanupFailureKind::Failed { detail },
                context.collector,
            );
            Some(advance(state, context))
        }
    }
}

fn retry(
    state: &mut CleanupState,
    detail: String,
    context: &mut TickContext<'_>,
) -> Option<RunState> {
    let pending = state.pending.as_ref()?;
    if pending.retries_used >= context.document.execution_policy.retry_limit {
        record_failure(
            state,
            CleanupFailureKind::RetryLimitExceeded {
                detail,
                retry_limit: context.document.execution_policy.retry_limit,
            },
            context.collector,
        );
        return Some(advance(state, context));
    }
    let previous_action_id = pending.directive.context().action_id;
    let retries_used = pending.retries_used.wrapping_add(1);
    let step = &state.steps[state.cursor];
    let retry = new_pending(
        context,
        step.phase_index,
        DirectivePurpose::Cleanup {
            cleanup_index: step.cleanup_index,
        },
        step.action.clone(),
        retries_used,
    );
    context
        .collector
        .events
        .push(EngineEvent::DirectiveRetried {
            previous_action_id,
            directive: retry.directive.clone(),
        });
    emit(&retry.directive, context.collector);
    state.pending = Some(retry);
    None
}

fn record_failure(
    state: &mut CleanupState,
    failure: CleanupFailureKind,
    collector: &mut super::runtime::TickCollector,
) {
    let step = &state.steps[state.cursor];
    let failure = CleanupFailure {
        phase_index: step.phase_index,
        phase_id: step.phase_id.clone(),
        cleanup_index: step.cleanup_index,
        action: step.action.name().to_owned(),
        failure,
    };
    collector.events.push(EngineEvent::CleanupFailed {
        failure: failure.clone(),
    });
    state.failures.push(failure);
}

fn advance(state: &mut CleanupState, context: &mut TickContext<'_>) -> RunState {
    state.cursor = state.cursor.wrapping_add(1);
    state.pending = None;
    issue_or_finish(state, context)
}

fn issue_or_finish(state: &mut CleanupState, context: &mut TickContext<'_>) -> RunState {
    let Some(step) = state.steps.get(state.cursor) else {
        return terminal_state(
            state.cause.clone().finish(state.failures.clone()),
            context.collector,
        );
    };
    let pending = new_pending(
        context,
        step.phase_index,
        DirectivePurpose::Cleanup {
            cleanup_index: step.cleanup_index,
        },
        step.action.clone(),
        0,
    );
    emit(&pending.directive, context.collector);
    state.pending = Some(pending);
    RunState::CleaningUp(Box::new(state.clone()))
}

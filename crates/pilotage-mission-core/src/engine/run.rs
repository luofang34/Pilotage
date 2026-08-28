//! Active-phase sequencing and terminal-cause selection.

use super::cleanup::start_cleanup;
use super::runtime::{
    PendingDirective, PendingTerminal, PhaseProgress, RunState, RunningState, TickContext, emit,
    new_pending, terminal_state,
};
use super::{
    AbortCause, DeadlineClass, DirectivePurpose, EngineEvent, MissionTerminal, ReceiptResult,
    WallDeadline,
};

pub(super) fn tick_running(
    wall_deadline: WallDeadline,
    state: &mut RunningState,
    mut context: TickContext<'_>,
) -> Option<RunState> {
    if context.input.wall_time_ns >= wall_deadline.expires_at_ns {
        let deadline = DeadlineClass::MissionWall {
            deadline_ns: wall_deadline.expires_at_ns,
            observed_ns: context.input.wall_time_ns,
        };
        context
            .collector
            .events
            .push(EngineEvent::DeadlineExceeded {
                deadline: deadline.clone(),
            });
        return Some(start_cleanup(
            PendingTerminal::DeadlineExceeded(deadline),
            &mut context,
        ));
    }
    let phase = &context.document.phases[state.phase_index];
    let elapsed = context
        .input
        .simulator_time_ns
        .saturating_sub(state.phase_started_at_ns);
    if elapsed >= phase.simulator_time_deadline_ns {
        return Some(phase_deadline(state.phase_index, elapsed, &mut context));
    }
    if let Some(index) = abort_condition(state.phase_index, &context) {
        return Some(abort(state.phase_index, index, &mut context));
    }
    let phase_started_at_ns = state.phase_started_at_ns;
    match &mut state.progress {
        PhaseProgress::Entry => enter_if_ready(state, &mut context),
        PhaseProgress::Receipt(pending) => process_receipt(
            state.phase_index,
            phase_started_at_ns,
            pending.as_mut(),
            &mut context,
        ),
        PhaseProgress::Completion => complete_if_ready(state, &mut context),
    }
}

fn enter_if_ready(state: &mut RunningState, context: &mut TickContext<'_>) -> Option<RunState> {
    let phase = &context.document.phases[state.phase_index];
    if !super::condition::all_match(
        &phase.entry_conditions,
        context.input.simulator_time_ns,
        &context.input.observation,
    ) {
        return None;
    }
    let phase_id = phase.id.clone();
    let action = phase.action.clone();
    context.entered_phases.push(state.phase_index);
    context.collector.events.push(EngineEvent::PhaseEntered {
        phase_index: state.phase_index,
        phase_id,
        simulator_time_ns: context.input.simulator_time_ns,
    });
    let pending = new_pending(
        context,
        state.phase_index,
        DirectivePurpose::PhaseAction {},
        action,
        0,
    );
    emit(&pending.directive, context.collector);
    state.progress = PhaseProgress::Receipt(Box::new(pending));
    None
}

fn process_receipt(
    phase_index: usize,
    phase_started_at_ns: u64,
    pending: &mut PendingDirective,
    context: &mut TickContext<'_>,
) -> Option<RunState> {
    let Some(receipt) = context.input.receipts.first() else {
        return receipt_timeout(phase_index, pending, context);
    };
    let result = receipt.result.clone();
    let directive_context = pending.directive.context().clone();
    context.collector.events.push(EngineEvent::ReceiptAccepted {
        context: directive_context.clone(),
        result: result.clone(),
    });
    match result {
        ReceiptResult::Succeeded {} => {
            complete_after_receipt(phase_index, phase_started_at_ns, context)
        }
        ReceiptResult::Retryable { detail } => retry_action(phase_index, pending, &detail, context),
        ReceiptResult::Refused { detail } => Some(refused(
            phase_index,
            directive_context,
            pending.directive.action_name(),
            detail,
            context,
        )),
        ReceiptResult::Failed { detail } => Some(action_failed(phase_index, detail, context)),
    }
}

fn complete_after_receipt(
    phase_index: usize,
    phase_started_at_ns: u64,
    context: &mut TickContext<'_>,
) -> Option<RunState> {
    let mut state = RunningState {
        phase_index,
        phase_started_at_ns,
        progress: PhaseProgress::Completion,
    };
    complete_if_ready(&mut state, context).or(Some(RunState::Running(state)))
}

fn retry_action(
    phase_index: usize,
    pending: &mut PendingDirective,
    detail: &str,
    context: &mut TickContext<'_>,
) -> Option<RunState> {
    if pending.retries_used >= context.document.execution_policy.retry_limit {
        let phase = &context.document.phases[phase_index];
        let cause = PendingTerminal::Aborted {
            phase_index,
            phase_id: phase.id.clone(),
            action: phase.action.name().to_owned(),
            cause: AbortCause::RetryLimitExceeded {
                detail: detail.to_owned(),
                retry_limit: context.document.execution_policy.retry_limit,
            },
        };
        return Some(start_cleanup(cause, context));
    }
    let previous_action_id = pending.directive.context().action_id;
    let retries_used = pending.retries_used.wrapping_add(1);
    let action = pending.directive.action();
    let retry = new_pending(
        context,
        phase_index,
        DirectivePurpose::PhaseAction {},
        action,
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
    *pending = retry;
    None
}

fn receipt_timeout(
    phase_index: usize,
    pending: &PendingDirective,
    context: &mut TickContext<'_>,
) -> Option<RunState> {
    let elapsed = context
        .input
        .wall_time_ns
        .saturating_sub(pending.emitted_at_wall_ns);
    if elapsed < context.document.execution_policy.receipt_timeout_ns {
        return None;
    }
    let directive_context = pending.directive.context().clone();
    let action = pending.directive.action_name().to_owned();
    context.collector.events.push(EngineEvent::ReceiptTimedOut {
        context: directive_context.clone(),
        action: action.clone(),
    });
    Some(start_cleanup(
        PendingTerminal::ReceiptTimeout {
            phase_index,
            phase_id: context.document.phases[phase_index].id.clone(),
            action,
            action_id: directive_context.action_id,
        },
        context,
    ))
}

fn complete_if_ready(state: &mut RunningState, context: &mut TickContext<'_>) -> Option<RunState> {
    let phase = &context.document.phases[state.phase_index];
    if !super::condition::all_match(
        &phase.completion_conditions,
        context.input.simulator_time_ns,
        &context.input.observation,
    ) {
        return None;
    }
    context.collector.events.push(EngineEvent::PhaseCompleted {
        phase_index: state.phase_index,
        phase_id: phase.id.clone(),
        simulator_time_ns: context.input.simulator_time_ns,
    });
    let next = state.phase_index.wrapping_add(1);
    if next == context.document.phases.len() {
        return Some(terminal_state(
            MissionTerminal::Complete {
                completed_phases: context.document.phases.len(),
            },
            context.collector,
        ));
    }
    state.phase_index = next;
    state.phase_started_at_ns = context.input.simulator_time_ns;
    state.progress = PhaseProgress::Entry;
    activate_new_phase(state, context)
}

fn activate_new_phase(state: &mut RunningState, context: &mut TickContext<'_>) -> Option<RunState> {
    if let Some(index) = abort_condition(state.phase_index, context) {
        return Some(abort(state.phase_index, index, context));
    }
    enter_if_ready(state, context)
}

fn abort_condition(phase_index: usize, context: &TickContext<'_>) -> Option<usize> {
    super::condition::first_match(
        &context.document.phases[phase_index].abort_conditions,
        context.input.simulator_time_ns,
        &context.input.observation,
    )
}

fn phase_deadline(phase_index: usize, elapsed_ns: u64, context: &mut TickContext<'_>) -> RunState {
    let phase = &context.document.phases[phase_index];
    let deadline = DeadlineClass::PhaseSimulatorTime {
        phase_index,
        phase_id: phase.id.clone(),
        limit_ns: phase.simulator_time_deadline_ns,
        elapsed_ns,
    };
    context
        .collector
        .events
        .push(EngineEvent::DeadlineExceeded {
            deadline: deadline.clone(),
        });
    start_cleanup(PendingTerminal::DeadlineExceeded(deadline), context)
}

fn abort(phase_index: usize, condition_index: usize, context: &mut TickContext<'_>) -> RunState {
    let phase = &context.document.phases[phase_index];
    let phase_id = phase.id.clone();
    let action = phase.action.name().to_owned();
    context
        .collector
        .events
        .push(EngineEvent::AbortConditionMatched {
            phase_index,
            phase_id: phase_id.clone(),
            condition_index,
        });
    start_cleanup(
        PendingTerminal::Aborted {
            phase_index,
            phase_id,
            action,
            cause: AbortCause::Condition { condition_index },
        },
        context,
    )
}

fn refused(
    phase_index: usize,
    directive_context: super::DirectiveContext,
    action: &str,
    detail: String,
    context: &mut TickContext<'_>,
) -> RunState {
    context.collector.events.push(EngineEvent::Refusal {
        context: directive_context,
        action: action.to_owned(),
        detail: detail.clone(),
    });
    start_cleanup(
        PendingTerminal::Refused {
            phase_index,
            phase_id: context.document.phases[phase_index].id.clone(),
            action: action.to_owned(),
            detail,
        },
        context,
    )
}

fn action_failed(phase_index: usize, detail: String, context: &mut TickContext<'_>) -> RunState {
    let phase = &context.document.phases[phase_index];
    let cause = PendingTerminal::Aborted {
        phase_index,
        phase_id: phase.id.clone(),
        action: phase.action.name().to_owned(),
        cause: AbortCause::ActionFailed { detail },
    };
    start_cleanup(cause, context)
}

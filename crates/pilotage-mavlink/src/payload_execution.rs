//! Turret lifecycle shared by an adapter and its final MAVLink writer.
use std::sync::{Arc, Mutex};
use std::time::Instant;
use turret_core::{
    Command, CommandIdentity, CommandProgress, Effect, Executor, PayloadCapabilities,
    PayloadCommand, Tick,
};

mod receipt;
mod write;
pub use receipt::WriteReceipt;
pub use write::PayloadWrite;

/// Failure to admit or access an executor-owned command.
#[derive(Debug, thiserror::Error)]
pub enum PayloadExecutionError {
    /// The shared executor cannot be accessed safely.
    #[error("payload executor lock poisoned")]
    Poisoned,
    /// The portable contract refused the operation.
    #[error("payload contract rejected operation: {0}")]
    Contract(#[from] turret_core::Error),
    /// The endpoint clock cannot represent the requested instant.
    #[error("payload deadline is outside the endpoint clock")]
    Clock,
}

/// Absolute deadlines preserved across host and transport queues.
#[derive(Debug, Clone, Copy)]
pub struct PayloadTiming {
    /// Exclusive deadline to begin writing.
    pub send_before: Instant,
    /// Execution deadline, required for continuous control.
    pub valid_until: Option<Instant>,
    /// Optional expected acknowledgement deadline.
    pub ack_before: Option<Instant>,
    /// Optional observed-completion deadline.
    pub completion_before: Option<Instant>,
}

/// Latest retained command evidence; targets are separate from device observations.
#[derive(Debug, Clone, Copy)]
pub struct PayloadSnapshot {
    /// The typed command whose evidence is retained.
    pub command: Command,
    /// Separate admission, send, acknowledgement, and termination facts.
    pub progress: CommandProgress,
    /// Endpoint-monotonic time of the latest lifecycle change.
    pub updated_at: Tick,
}

#[derive(Debug)]
struct State {
    executor: Executor,
    sequence: u64,
    command: Option<Command>,
    snapshot: Option<PayloadSnapshot>,
    effect: Effect,
}

impl State {
    fn note(&mut self, now: Tick) {
        if let Some(command) = self.command
            && let Some(progress) = self.executor.progress(command.identity)
        {
            let changed = self
                .snapshot
                .is_none_or(|snapshot| snapshot.progress != progress);
            if changed {
                self.snapshot = Some(PayloadSnapshot {
                    command,
                    progress,
                    updated_at: now,
                });
            }
        }
    }

    fn invalidated(&mut self, invalidation: turret_core::Invalidation, now: Tick) {
        if let Some(active) = invalidation.active {
            self.effect(active.effect);
        }
        self.note(now);
        if let Some(command) = self.command
            && let Some(progress) = invalidation
                .pending
                .filter(|pending| pending.identity == command.identity)
        {
            self.snapshot = Some(PayloadSnapshot {
                command,
                progress,
                updated_at: now,
            });
        }
    }

    fn effect(&mut self, effect: Effect) {
        if effect == Effect::StopContinuous || self.effect == Effect::None {
            self.effect = effect;
        }
    }
}

/// Bounded endpoint executor; I/O stays with the MAVLink link task.
#[derive(Debug, Clone)]
pub struct PayloadChannel {
    state: Arc<Mutex<State>>,
    started: Instant,
}

impl PayloadChannel {
    /// Create a channel for one gimbal or one camera connection epoch.
    #[must_use]
    pub fn new(capabilities: PayloadCapabilities, epoch: u64, started: Instant) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                executor: Executor::new(capabilities, epoch, 0),
                sequence: 0,
                command: None,
                snapshot: None,
                effect: Effect::None,
            })),
            started,
        }
    }

    /// Admit one command with the caller's original deadlines.
    pub fn submit(
        &self,
        payload: PayloadCommand,
        id: u64,
        timing: PayloadTiming,
    ) -> Result<PayloadWrite, PayloadExecutionError> {
        let timing = turret_core::CommandTiming {
            send_before: self.tick(timing.send_before)?,
            valid_until: timing.valid_until.map(|at| self.tick(at)).transpose()?,
            ack_before: timing.ack_before.map(|at| self.tick(at)).transpose()?,
            completion_before: timing
                .completion_before
                .map(|at| self.tick(at))
                .transpose()?,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| PayloadExecutionError::Poisoned)?;
        let now = self.tick(Instant::now())?;
        state.sequence = state.sequence.wrapping_add(1);
        let identity = CommandIdentity {
            epoch: state.executor.epoch(),
            generation: state.executor.generation(),
            id,
            sequence: state.sequence,
        };
        let command = Command {
            identity,
            capability_revision: state.executor.capabilities().revision,
            payload,
            timing,
        };
        state.executor.submit(command, now)?;
        state.command = Some(command);
        state.note(now);
        Ok(PayloadWrite {
            channel: self.clone(),
            identity,
        })
    }

    /// Fence queued writes and retain the stop/cancel request for the adapter.
    pub fn revoke(&self) -> Result<(), PayloadExecutionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PayloadExecutionError::Poisoned)?;
        let now = self.tick(Instant::now())?;
        let invalidation = state.executor.revoke();
        state.invalidated(invalidation, now);
        Ok(())
    }

    /// Replace a changed capability contract without inventing a connection reset.
    pub fn update_capabilities(
        &self,
        capabilities: PayloadCapabilities,
    ) -> Result<(), PayloadExecutionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PayloadExecutionError::Poisoned)?;
        let now = self.tick(Instant::now())?;
        if state.executor.capabilities() == &capabilities {
            return Ok(());
        }
        let invalidation = state.executor.update_capabilities(capabilities);
        state.invalidated(invalidation, now);
        Ok(())
    }

    /// Advance lifecycle deadlines and take the strongest pending device effect.
    pub fn poll_effect(&self) -> Result<Effect, PayloadExecutionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PayloadExecutionError::Poisoned)?;
        let now = self.tick(Instant::now())?;
        while let Some(event) = state.executor.poll(now)? {
            state.effect(event.effect);
        }
        state.note(now);
        Ok(std::mem::replace(&mut state.effect, Effect::None))
    }

    /// Retained command evidence; reading it does not refresh any timestamp.
    pub fn snapshot_cached(&self) -> Result<Option<PayloadSnapshot>, PayloadExecutionError> {
        let state = self
            .state
            .lock()
            .map_err(|_| PayloadExecutionError::Poisoned)?;
        Ok(state.snapshot)
    }

    fn tick(&self, at: Instant) -> Result<Tick, PayloadExecutionError> {
        at.checked_duration_since(self.started)
            .and_then(|duration| u64::try_from(duration.as_micros()).ok())
            .map(Tick)
            .ok_or(PayloadExecutionError::Clock)
    }
}

#[cfg(test)]
mod tests;

//! The Turret lifecycle at the final DJI transport boundary.

use std::time::Duration;

use turret_core::{
    Angle, Axes, Command, CommandIdentity, CommandProgress, CommandTiming, Effect, Executor,
    GimbalPosition, PayloadCapabilities, PayloadCommand, ReferenceFrame, Tick,
};

use crate::GimbalPitchTarget;

pub(super) struct Execution {
    executor: Executor,
    command: Option<Command>,
    progress: Option<CommandProgress>,
    sequence: u64,
    now: Tick,
    wire_sequence: u16,
    awaiting_response: Option<(CommandIdentity, u16, Tick)>,
}

impl Execution {
    pub(super) fn new(capabilities: PayloadCapabilities, epoch: u64) -> Self {
        Self {
            executor: Executor::new(capabilities, epoch, 0),
            command: None,
            progress: None,
            sequence: 0,
            now: Tick(0),
            wire_sequence: 0,
            awaiting_response: None,
        }
    }

    pub(super) fn capabilities(&self) -> &PayloadCapabilities {
        self.executor.capabilities()
    }

    pub(super) fn update_capabilities(&mut self, capabilities: PayloadCapabilities) {
        self.awaiting_response = None;
        let invalidation = self.executor.update_capabilities(capabilities);
        if let Some(active) = invalidation.active {
            report_effect(active.effect);
        }
        self.refresh();
        if let Some(pending) = invalidation.pending {
            self.progress = Some(pending);
        }
    }

    pub(super) fn admit(
        &mut self,
        pitch: GimbalPitchTarget,
        wire_sequence: u16,
        now: Duration,
        send_before: Duration,
    ) -> Result<CommandIdentity, turret_core::Error> {
        let command = Command {
            identity: CommandIdentity {
                epoch: self.executor.epoch(),
                generation: self.executor.generation(),
                id: self.sequence,
                sequence: self.sequence,
            },
            capability_revision: self.executor.capabilities().revision,
            payload: PayloadCommand::GimbalPosition(GimbalPosition {
                angles: Axes {
                    pitch: Some(Angle::radians(
                        f32::from(pitch.tenths()).to_radians() / 10.0,
                    )?),
                    roll: None,
                    yaw: None,
                },
                frame: ReferenceFrame::Mount,
            }),
            timing: CommandTiming {
                send_before: tick(send_before),
                // The device receives a position, not a continuing rate. It has
                // no verified cancellation, ACK, or completion feedback path.
                valid_until: None,
                ack_before: None,
                completion_before: None,
            },
        };
        self.now = tick(now);
        self.executor.submit(command, self.now)?;
        self.sequence = self.sequence.wrapping_add(1);
        self.command = Some(command);
        self.wire_sequence = wire_sequence;
        self.progress = self.executor.progress(command.identity);
        Ok(command.identity)
    }

    pub(super) fn prepare(&mut self, now: Duration) -> Result<(), turret_core::Error> {
        self.now = tick(now);
        let result = self.executor.prepare_send(self.now);
        self.refresh();
        result?.ok_or(turret_core::Error::NotFound)?;
        Ok(())
    }

    pub(super) fn sent(
        &mut self,
        identity: CommandIdentity,
        now: Duration,
    ) -> Result<(), turret_core::Error> {
        self.now = tick(now);
        let outcome = self.executor.record_sent(identity, self.now)?;
        self.awaiting_response = Some((identity, self.wire_sequence, self.now));
        self.progress = Some(outcome.current.progress);
        report_effect(outcome.current.effect);
        Ok(())
    }

    pub(super) fn failed(&mut self, identity: CommandIdentity) {
        self.awaiting_response = None;
        match self.executor.record_write_failure(identity, self.now) {
            Ok(failure) => {
                self.progress = Some(failure.attempted.progress);
                report_effect(failure.attempted.effect);
                if let Some(active) = failure.active {
                    report_effect(active.effect);
                }
            }
            Err(error) => tracing::error!(%error, "DJI write evidence could not be retained"),
        }
    }

    pub(super) fn cancel(&mut self) {
        self.awaiting_response = None;
        let invalidation = self.executor.revoke();
        if let Some(active) = invalidation.active {
            report_effect(active.effect);
        }
        self.refresh();
        if let Some(pending) = invalidation.pending {
            self.progress = Some(pending);
        }
    }

    pub(super) fn response(&mut self, frame: &crate::DumlFrame, now: Duration) {
        let Some((identity, sequence, sent_at)) = self.awaiting_response else {
            return;
        };
        let now = tick(now);
        // A short correlation window limits ambiguity of the 16-bit wire sequence.
        // It is local acceptance policy, not a measured device response guarantee.
        if now < sent_at || now.0.saturating_sub(sent_at.0) > 1_000_000 {
            self.awaiting_response = None;
            return;
        }
        let Some(code) = crate::pitch_response(frame, sequence) else {
            return;
        };
        self.awaiting_response = None;
        if code != 0 {
            tracing::warn!(
                code,
                sequence,
                "DJI command returned an unverified response code"
            );
            return;
        }
        self.now = now;
        match self.executor.acknowledge(identity, true, now) {
            Ok(event) => {
                report_effect(event.effect);
                self.refresh();
            }
            Err(error) => tracing::debug!(%error, "DJI response does not match active execution"),
        }
    }

    pub(super) fn wire_progress(&self) -> Option<pilotage_protocol::wire::PayloadCommandProgress> {
        self.progress.as_ref().map(|progress| {
            let mut wire = pilotage_adapter_api::payload_progress_to_wire(
                progress, self.command.as_ref(), self.now,
            );
            if progress.sent_at.is_some() {
                wire.detail = if progress.acknowledged_at.is_some() {
                    "DJI returned a matching zero response; measured completion and physical stop are unverified"
                } else {
                    "Written to DJI transport; device acceptance, completion and physical stop are unverified"
                }.to_owned();
            }
            wire
        })
    }

    fn refresh(&mut self) {
        if let Some(command) = self.command
            && let Some(progress) = self.executor.progress(command.identity)
        {
            self.progress = Some(progress);
        }
    }
}

pub(super) fn tick(time: Duration) -> Tick {
    Tick(time.as_micros().min(u128::from(u64::MAX)) as u64)
}

fn report_effect(effect: Effect) {
    if effect != Effect::None {
        tracing::debug!(
            ?effect,
            "DJI future writes revoked; an emitted position may still finish"
        );
    }
}

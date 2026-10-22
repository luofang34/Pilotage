use std::time::Duration;

use pilotage_adapter_api::RejectReason;

use super::{Connection, DjiFault, Pending, Purpose};
use crate::adapter::{motion::MotionInput, observation::Observations};
use crate::flight::{
    VirtualStickStream, VirtualStickStreamPhase as Phase, VirtualStickVelocity, VirtualStickWrite,
};

impl Connection {
    pub(in crate::adapter) fn accept_motion(
        &mut self,
        input: MotionInput,
        now: Duration,
        observations: &Observations,
        wire_sequence: &mut u16,
    ) -> Result<(), RejectReason> {
        let MotionInput {
            binding,
            sequence,
            value,
            until,
        } = input;
        if !self.input_clear()
            || !self.remote_control.neutral_cached(now)
            || !self.version_observed
            || self.flight.active()
            || observations.model_deadline(now).is_none()
        {
            return Err(RejectReason::MeasurementUnavailable);
        }
        if observations
            .motion_deadline(now, self.mode_session.retained())
            .is_none()
        {
            return Err(RejectReason::MeasurementUnavailable);
        }
        self.bind_motion(input, now, observations, wire_sequence)?;
        if !self.mode_session.retain(now, until) {
            self.motion.stop(now);
            return Err(RejectReason::Fenced);
        }
        if let Some(stream) = &mut self.motion.stream {
            stream
                .submit(binding, sequence, value, now, until)
                .map_err(|error| RejectReason::Other(error.to_string()))?;
        }
        self.motion.sequence = Some(sequence);
        self.motion.latest = Some((value, until));
        Ok(())
    }

    fn bind_motion(
        &mut self,
        input: MotionInput,
        now: Duration,
        observations: &Observations,
        wire_sequence: &mut u16,
    ) -> Result<(), RejectReason> {
        let MotionInput {
            binding,
            sequence,
            value,
            until,
        } = input;
        if let Some(current) = self.motion.binding {
            if current != binding {
                self.motion.stop(now);
                let forward = current.session != binding.session
                    || (binding.generation.wrapping_sub(current.generation) != 0
                        && binding.generation.wrapping_sub(current.generation) < 1 << 63);
                if !forward || self.mode_session.active() || self.mode_session.unresolved() {
                    return Err(RejectReason::Fenced);
                }
                self.motion = crate::adapter::motion::Motion::default();
                return self.bind_motion(input, now, observations, wire_sequence);
            }
            if self.motion.stopped
                || self.motion.sequence.is_some_and(|previous| {
                    let delta = sequence.wrapping_sub(previous);
                    delta == 0 || delta >= 1 << 31
                })
            {
                return Err(RejectReason::Fenced);
            }
        } else {
            if value != VirtualStickVelocity::ZERO
                || self.pending.is_some()
                || self.keyframes.active.is_some()
                || !self.mode_session.reset_released()
            {
                return Err(RejectReason::Fenced);
            }
            let fence = self
                .mode_session
                .begin(1, binding.model_revision, *wire_sequence, now, until)
                .map_err(|error| RejectReason::Other(error.to_string()))?;
            if !self.mode_session.retain(now, until) {
                return Err(RejectReason::Fenced);
            }
            self.pending = Some(
                self.mode_pending(fence, observations, now)
                    .map_err(|error| RejectReason::Other(error.to_string()))?,
            );
            *wire_sequence = wire_sequence.wrapping_add(2);
            self.motion.binding = Some(binding);
            self.motion.stopped = false;
        }
        Ok(())
    }

    pub(in crate::adapter) fn drive_motion(
        &mut self,
        observations: &Observations,
        now: Duration,
        sequence: &mut u16,
    ) {
        let Some(binding) = self.motion.binding else {
            return;
        };
        if observations.model_revision() != binding.model_revision
            || observations.model_deadline(now).is_none()
        {
            self.motion.lost();
            self.mode_session.source_changed();
            self.cancel_stick_pending(now);
            return;
        }
        if observations
            .motion_deadline(now, self.mode_session.retained())
            .is_none()
        {
            self.motion.stop(now);
            self.cancel_stick_pending(now);
            self.mode_session.release_retained(now, None);
            return;
        }
        if self.motion.latest.is_none_or(|(_, until)| now >= until) || self.flight.active() {
            self.motion.stop(now);
        }
        if !self.mode_session.enabled() {
            if self.motion.stream.is_some() || self.motion.stopped {
                self.motion.stop(now);
                self.cancel_stick_pending(now);
                self.mode_session.release_retained(now, None);
            }
            return;
        }
        self.queue_stick(binding, now, sequence);
    }

    fn queue_stick(
        &mut self,
        binding: crate::flight::VirtualStickBinding,
        now: Duration,
        sequence: &mut u16,
    ) {
        if self.motion.stream.is_none() && !self.motion.stopped {
            let mut stream = VirtualStickStream::new(binding, now);
            if let Some((value, until)) = self.motion.latest
                && let Some(input_sequence) = self.motion.sequence
                && stream
                    .submit(binding, input_sequence, value, now, until)
                    .is_ok()
            {
                self.motion.stream = Some(stream);
            } else {
                self.motion.stop(now);
            }
        }
        self.cancel_invalid_stick(now);
        if self.fault.is_some() {
            return;
        }
        if let Some(stream) = &self.motion.stream {
            if matches!(stream.phase(), Phase::ReleaseRequired | Phase::Uncertain) {
                self.mode_session.release_retained(now, None);
                return;
            }
        } else if self.motion.stopped {
            self.mode_session.release_retained(now, None);
            return;
        }
        if self.pending.is_some() || self.keyframes.active.is_some() || !self.input_clear() {
            return;
        }
        if let Some(write) = self
            .motion
            .stream
            .as_mut()
            .and_then(|stream| stream.next_write(now, *sequence))
        {
            self.pending = Some(Pending {
                bytes: self.input.framing().command(&write.packet()),
                offset: 0,
                purpose: Purpose::Stick(write),
                expires_at: Some(write.send_until()),
            });
            *sequence = sequence.wrapping_add(1);
        }
    }

    fn cancel_invalid_stick(&mut self, now: Duration) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| match pending.purpose {
                Purpose::Stick(write) => !self
                    .motion
                    .stream
                    .as_ref()
                    .is_some_and(|stream| stream.permits(write, now)),
                _ => false,
            })
        {
            self.cancel_stick_pending(now);
        }
    }

    pub(in crate::adapter) fn cancel_stick_pending(&mut self, now: Duration) {
        if let Some(pending) = &self.pending
            && let Purpose::Stick(write) = pending.purpose
        {
            if pending.offset > 0 {
                self.fail(DjiFault::PartialCommand);
                return;
            }
            if let Some(stream) = &mut self.motion.stream {
                stream.discard_unwritten(write, now);
            }
            self.pending = None;
        }
    }

    pub(super) fn prepare_stick(
        &mut self,
        write: VirtualStickWrite,
        observations: &Observations,
        now: Duration,
    ) -> bool {
        let permitted = self.input_clear()
            && self.remote_control.neutral_cached(now)
            && self.mode_session.enabled()
            && self
                .motion
                .binding
                .is_some_and(|binding| binding.model_revision == observations.model_revision())
            && observations.motion_deadline(now, true).is_some()
            && self
                .motion
                .stream
                .as_ref()
                .is_some_and(|stream| stream.permits(write, now));
        if !permitted {
            self.motion.stop(now);
            self.cancel_stick_pending(now);
        }
        permitted
    }
}

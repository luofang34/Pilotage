use std::time::Duration;

use super::{Connection, DjiFault, Pending, Purpose};
use crate::adapter::{
    observation::Observations,
    virtual_stick::{DjiVirtualStickDiagnosticError as Error, ModeFence, ModeSession},
};
use crate::flight::{BasicFlightCommand, VirtualStickMode};

impl Connection {
    pub(in crate::adapter) fn queue_mode_session(
        &mut self,
        request_id: u64,
        sequence: u16,
        now: Duration,
        send_until: Duration,
        observations: &Observations,
    ) -> Result<(), Error> {
        if !self.input_clear()
            || !self.version_observed
            || self.pending.is_some()
            || self.keyframes.active.is_some()
            || self.flight.active()
        {
            return Err(Error::Busy);
        }
        observations
            .flight_deadline(BasicFlightCommand::Takeoff, now)
            .ok_or(Error::MissingFeedback)?;
        let fence = self.mode_session.begin(
            request_id,
            observations.model_revision(),
            sequence,
            now,
            send_until,
        )?;
        self.pending = Some(self.mode_pending(fence, observations, now)?);
        Ok(())
    }

    pub(super) fn mode_pending(
        &self,
        fence: ModeFence,
        observations: &Observations,
        now: Duration,
    ) -> Result<Pending, Error> {
        let until = observations
            .model_deadline(now)
            .ok_or(Error::MissingFeedback)?;
        let packet = ModeSession::packet(&fence);
        Ok(Pending {
            bytes: self.input.framing().command(packet.bytes()),
            offset: 0,
            purpose: Purpose::Mode(fence),
            expires_at: Some(fence.send_until.min(until)),
        })
    }

    pub(in crate::adapter) fn drive_mode_session(
        &mut self,
        observations: &Observations,
        now: Duration,
    ) {
        let binding = observations
            .model_deadline(now)
            .map(|_| observations.model_revision());
        self.mode_session.poll(now, binding);
        let valid = self
            .pending
            .as_ref()
            .is_none_or(|pending| match &pending.purpose {
                Purpose::Mode(fence) => self.mode_session.permits(fence, now),
                _ => true,
            });
        if !valid && !self.cancel_mode_pending() {
            return;
        }
        if self.fault.is_some() || !self.input_clear() {
            return;
        }
        if self.pending.is_some() {
            return;
        }
        let Some(fence) = self.mode_session.cleanup_fence() else {
            return;
        };
        if self.pending.is_some() {
            self.mode_session.transport_failed();
            return;
        }
        match self.mode_pending(fence, observations, now) {
            Ok(pending) => self.pending = Some(pending),
            Err(_) => self.mode_session.source_changed(),
        }
    }

    pub(super) fn prepare_mode(
        &mut self,
        fence: &ModeFence,
        observations: &Observations,
        now: Duration,
    ) -> bool {
        let fresh = observations.model_revision() == fence.model_revision
            && observations.model_deadline(now).is_some()
            && (fence.mode == VirtualStickMode::Disable
                || (self.mode_session.retained()
                    && observations.motion_deadline(now, false).is_some())
                || observations
                    .flight_deadline(BasicFlightCommand::Takeoff, now)
                    .is_some());
        if self.input_clear()
            && self.version_observed
            && fresh
            && self.mode_session.permits(fence, now)
        {
            let until = if fence.mode == VirtualStickMode::Enable && !self.mode_session.retained() {
                observations.flight_deadline(BasicFlightCommand::Takeoff, now)
            } else {
                observations.model_deadline(now)
            };
            if let Some(pending) = &mut self.pending {
                pending.expires_at = until.map(|until| until.min(fence.send_until));
            }
            return true;
        }
        self.mode_session.cancel();
        self.cancel_mode_pending();
        false
    }

    pub(super) fn cancel_mode_pending(&mut self) -> bool {
        if let Some(pending) = &self.pending
            && matches!(pending.purpose, Purpose::Mode(_))
        {
            if pending.offset > 0 {
                self.fail(DjiFault::PartialCommand);
                return false;
            }
            self.pending = None;
        }
        true
    }
}

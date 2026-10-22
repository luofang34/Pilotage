use std::time::Duration;

use pilotage_adapter_api::{
    CommandOutcome, FlightProcedurePhase as Phase, FlightProcedureReason as Reason,
};

use super::{ACK_TIMEOUT, FlightExecution, FlightFence, terminal, timestamp};
use crate::{DumlFrame, flight::BasicFlightCommand};

impl FlightExecution {
    pub(in crate::adapter) fn poll(&mut self, now: Duration) {
        self.now = now;
        self.poll_landing_override(now);
        let Some(procedure) = &self.procedure else {
            return;
        };
        if terminal(procedure.sample.phase) {
            return;
        }
        let until = if procedure.sample.phase == Phase::Queued {
            procedure.fence.send_until
        } else {
            procedure
                .ack_until
                .unwrap_or(procedure.completion_until)
                .min(procedure.completion_until)
        };
        if now >= until {
            self.transition(Phase::TimedOut, Some(Reason::DeadlineExpired), None, now);
        }
    }

    pub(in crate::adapter) fn wrote(&mut self, fence: &FlightFence, now: Duration, complete: bool) {
        self.now = now;
        let Some(procedure) = &mut self.procedure else {
            return;
        };
        if procedure.fence != *fence {
            return;
        }
        if procedure.command == BasicFlightCommand::Takeoff {
            self.takeoff_uncertain = true;
        }
        if complete {
            procedure.written = true;
            procedure.ack_until = Some(now.saturating_add(ACK_TIMEOUT));
            if now >= fence.send_until {
                self.transition(Phase::TimedOut, Some(Reason::DeadlineExpired), None, now);
            } else {
                self.transition(Phase::Written, None, None, now);
            }
        }
    }

    pub(in crate::adapter) fn response(&mut self, frame: &DumlFrame, now: Duration) {
        self.poll(now);
        self.confirmation.receive(frame, now);
        if self.confirmation.response(frame) {
            return;
        }
        if self.landing_override_response(frame, now) {
            return;
        }
        let Some(procedure) = &mut self.procedure else {
            return;
        };
        let Some(code) = procedure.packet.response(frame) else {
            return;
        };
        if !procedure.written
            || terminal(procedure.sample.phase)
            || procedure.sample.fc_outcome.is_some()
        {
            return;
        }
        tracing::info!(
            action_id = procedure.sample.request.action_id,
            sequence = frame.sequence(),
            code,
            "DJI flight command response; this does not establish physical completion"
        );
        procedure.ack_until = None;
        if code == 0 {
            self.transition(Phase::InProgress, None, Some(CommandOutcome::Accepted), now);
        } else {
            self.transition(
                Phase::Failed,
                Some(Reason::CommandRefused),
                Some(CommandOutcome::Failed),
                now,
            );
        }
    }

    pub(in crate::adapter) fn invalidate(&mut self, reason: Reason, now: Duration) {
        self.now = now;
        if reason == Reason::AuthorityLost {
            self.revoked = true;
        }
        if self
            .procedure
            .as_ref()
            .is_some_and(|procedure| !terminal(procedure.sample.phase))
        {
            self.transition(Phase::Cancelled, Some(reason), None, now);
        }
    }

    pub(in crate::adapter) fn transport_failed(&mut self) {
        if let Some(procedure) = &self.procedure
            && !terminal(procedure.sample.phase)
        {
            // A failed submission has no reliable no-write receipt.
            if procedure.command == BasicFlightCommand::Takeoff {
                self.takeoff_uncertain = true;
            }
            self.transition(
                Phase::OutcomeUnknown,
                Some(Reason::WriteFailed),
                None,
                self.now,
            );
        }
    }

    pub(super) fn transition(
        &mut self,
        phase: Phase,
        reason: Option<Reason>,
        outcome: Option<CommandOutcome>,
        now: Duration,
    ) {
        let Some(procedure) = &mut self.procedure else {
            return;
        };
        let outcome = outcome.or(procedure.sample.fc_outcome);
        if (
            procedure.sample.phase,
            procedure.sample.reason,
            procedure.sample.fc_outcome,
        ) == (phase, reason, outcome)
        {
            return;
        }
        procedure.sample.phase = phase;
        procedure.sample.reason = reason;
        procedure.sample.fc_outcome = outcome;
        procedure.sample.revision = procedure.sample.revision.wrapping_add(1);
        procedure.sample.changed_at = timestamp(now);
    }
}

use std::time::Duration;

use pilotage_adapter_api::{
    CommandOutcome, FlightProcedurePhase as Phase, FlightProcedureReason as Reason,
};

use super::{FlightExecution, FlightFence};
use crate::{
    DumlFrame,
    flight::{BasicFlightCommand, BasicFlightPacket, VirtualStickVelocity},
};

pub(super) struct LandingOverride {
    pub(super) fence: FlightFence,
    packet: BasicFlightPacket,
    written: bool,
    finished: bool,
    accepted_at: Option<Duration>,
    complete_until: Duration,
}

impl FlightExecution {
    pub(in crate::adapter) fn landing_override_input(
        &mut self,
        input: crate::adapter::motion::MotionInput,
        now: Duration,
        sequence: u16,
    ) -> Result<Option<(FlightFence, BasicFlightPacket)>, pilotage_adapter_api::RejectReason> {
        let Some(procedure) = self.procedure.as_mut() else {
            return Ok(None);
        };
        let delta = input.sequence.wrapping_sub(procedure.manual_sequence);
        if delta == 0 || delta >= 1 << 31 {
            return Err(pilotage_adapter_api::RejectReason::Fenced);
        }
        procedure.manual_sequence = input.sequence;
        if procedure.sample.phase != Phase::InProgress
            || procedure.sample.fc_outcome != Some(CommandOutcome::Accepted)
            || procedure.override_control.is_some()
        {
            return Ok(None);
        }
        if input.value == VirtualStickVelocity::ZERO {
            procedure.manual_ready = true;
            return Ok(None);
        }
        if !procedure.manual_ready {
            return Ok(None);
        }
        let mut fence = procedure.fence.clone();
        fence.send_until = input.until.min(now.saturating_add(super::MAX_SEND_DELAY));
        let packet = BasicFlightCommand::CancelLanding.packet(sequence);
        procedure.override_control = Some(LandingOverride {
            fence: fence.clone(),
            packet,
            written: false,
            finished: false,
            accepted_at: None,
            complete_until: now.saturating_add(Duration::from_secs(2)),
        });
        Ok(Some((fence, packet)))
    }

    pub(in crate::adapter) fn permits_landing_override(
        &self,
        fence: &FlightFence,
        model_revision: u64,
        now: Duration,
    ) -> bool {
        !self.revoked
            && self.procedure.as_ref().is_some_and(|procedure| {
                procedure.sample.phase == Phase::InProgress
                    && fence.model_revision == model_revision
                    && fence.incarnation == self.incarnation
                    && now < fence.send_until
                    && procedure.override_control.as_ref().is_some_and(|control| {
                        control.fence == *fence && !control.written && !control.finished
                    })
            })
    }

    pub(in crate::adapter) fn landing_override_written(&mut self, now: Duration) {
        if let Some(control) = self
            .procedure
            .as_mut()
            .and_then(|p| p.override_control.as_mut())
        {
            control.written = now < control.fence.send_until;
        }
    }

    pub(super) fn landing_override_response(&mut self, frame: &DumlFrame, now: Duration) -> bool {
        let Some(procedure) = &mut self.procedure else {
            return false;
        };
        let Some(control) = &mut procedure.override_control else {
            return false;
        };
        let Some(code) = control.packet.response(frame) else {
            return false;
        };
        if control.finished
            || !control.written
            || control.accepted_at.is_some()
            || procedure.sample.phase != Phase::InProgress
        {
            return true;
        }
        if code == 0 {
            control.accepted_at = Some(now);
        } else {
            control.finished = true;
            tracing::warn!(
                code,
                "DJI declined landing cancellation; landing remains active"
            );
        }
        true
    }

    pub(super) fn observe_landing_override(
        &mut self,
        report: crate::flight::Mini3FlightReport,
        now: Duration,
    ) -> bool {
        let Some(procedure) = &mut self.procedure else {
            return false;
        };
        let Some(control) = &procedure.override_control else {
            return false;
        };
        if !report.is_flying && !report.motors_running {
            return false;
        }
        if procedure.sample.phase == Phase::InProgress
            && !control.finished
            && control.accepted_at.is_some_and(|at| now > at)
            && report.mode == 6
            && report.is_flying
            && report.motors_running
        {
            procedure.manual_released = true;
            self.transition(Phase::Cancelled, Some(Reason::Interrupted), None, now);
        }
        true
    }

    pub(super) fn poll_landing_override(&mut self, now: Duration) {
        let Some(control) = self.procedure.as_mut().and_then(|p| {
            (p.sample.phase == Phase::InProgress)
                .then_some(p.override_control.as_mut())
                .flatten()
        }) else {
            return;
        };
        if !control.finished
            && (now >= control.complete_until
                || (!control.written && now >= control.fence.send_until))
        {
            // An uncertain cancel must not hide a later landing completion or retry.
            control.finished = true;
            tracing::warn!("DJI landing cancellation timed out; continue observing landing");
        }
    }

    pub(in crate::adapter) fn take_landing_override(
        &mut self,
        binding: crate::flight::VirtualStickBinding,
    ) -> bool {
        if !self.matches_landing_authority(binding) {
            return false;
        }
        let Some(procedure) = &mut self.procedure else {
            return false;
        };
        if !procedure.manual_released || procedure.manual_resumed {
            return false;
        }
        procedure.manual_resumed = true;
        true
    }
}

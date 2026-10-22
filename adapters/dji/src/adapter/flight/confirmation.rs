use std::time::Duration;

use pilotage_adapter_api::{CommandOutcome, FlightProcedurePhase};

use super::{FlightExecution, FlightFence};
use crate::{
    DumlFrame, RouteAddress,
    flight::{BasicFlightCommand, BasicFlightPacket},
};

#[derive(Default)]
pub(super) struct Confirmation {
    report: Option<(u16, bool, Duration)>,
    hold: Option<(Duration, Duration)>,
    command: Option<(FlightFence, BasicFlightPacket, bool)>,
}

impl Confirmation {
    pub(super) fn receive(&mut self, frame: &DumlFrame, now: Duration) {
        if frame.version() != 1
            || frame.sender() != RouteAddress(9)
            || frame.receiver() != RouteAddress(2)
            || frame.flags() != 0
            || frame.command_set() != 3
            || frame.command_id() != 0x53
        {
            return;
        }
        let [flags, _] = frame.payload() else {
            self.report = None;
            self.hold = None;
            return;
        };
        if self.report.is_some_and(|(sequence, _, _)| {
            let delta = frame.sequence().wrapping_sub(sequence);
            delta == 0 || delta >= 0x8000
        }) {
            return;
        }
        self.report = Some((frame.sequence(), flags & 0x20 != 0, now));
    }

    fn needed(&self, now: Duration) -> bool {
        self.report.is_some_and(|(_, needed, at)| {
            needed && now >= at && now - at < Duration::from_millis(300)
        })
    }

    pub(super) fn response(&self, frame: &DumlFrame) -> bool {
        let Some((_, packet, true)) = &self.command else {
            return false;
        };
        let Some(code) = packet.response(frame) else {
            return false;
        };
        tracing::info!(
            code,
            "DJI landing confirmation response; touchdown still requires observation"
        );
        true
    }
}

impl FlightExecution {
    pub(in crate::adapter) fn confirm_landing_input(
        &mut self,
        input: crate::adapter::motion::MotionInput,
        now: Duration,
        sequence: u16,
    ) -> Option<(FlightFence, BasicFlightPacket)> {
        let procedure = self.procedure.as_ref()?;
        let ready = !self.revoked
            && self.confirmation.needed(now)
            && procedure.sample.phase == FlightProcedurePhase::InProgress
            && procedure.sample.fc_outcome == Some(CommandOutcome::Accepted)
            && procedure.override_control.is_none()
            && self.confirmation.command.is_none()
            && input.value.is_descent_only();
        if !ready {
            self.confirmation.hold = None;
            return None;
        }
        let (start, previous) = self.confirmation.hold.unwrap_or((now, now));
        let start = if now.saturating_sub(previous) > Duration::from_millis(150) {
            now
        } else {
            start
        };
        self.confirmation.hold = Some((start, now));
        if now.saturating_sub(start) < Duration::from_secs(2) {
            return None;
        }
        let mut fence = procedure.fence.clone();
        fence.send_until = input.until.min(now.saturating_add(super::MAX_SEND_DELAY));
        let packet = BasicFlightCommand::ConfirmLanding.packet(sequence);
        self.confirmation.command = Some((fence.clone(), packet, false));
        Some((fence, packet))
    }

    pub(in crate::adapter) fn permits_landing_confirmation(
        &self,
        fence: &FlightFence,
        model: u64,
        now: Duration,
    ) -> bool {
        !self.revoked
            && self.confirmation.needed(now)
            && fence.model_revision == model
            && fence.incarnation == self.incarnation
            && now < fence.send_until
            && self.procedure.as_ref().is_some_and(|p| {
                p.sample.phase == FlightProcedurePhase::InProgress && p.override_control.is_none()
            })
            && self
                .confirmation
                .command
                .as_ref()
                .is_some_and(|(current, _, written)| current == fence && !written)
    }

    pub(in crate::adapter) fn landing_confirmation_written(&mut self) {
        if let Some((_, _, written)) = &mut self.confirmation.command {
            *written = true;
        }
    }
}

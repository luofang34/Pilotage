use std::time::Duration;

use pilotage_adapter_api::FlightProcedurePhase;
use pilotage_protocol::ScopedControlFrame;

use super::{FlightExecution, FlightFence, terminal};
use crate::flight::BasicFlightPacket;

const HANDOVER_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct LandingPreparation {
    until: Duration,
    started: bool,
}

impl FlightExecution {
    pub(in crate::adapter) fn matches_landing_authority(
        &self,
        binding: crate::flight::VirtualStickBinding,
    ) -> bool {
        !self.revoked
            && self.procedure.as_ref().is_some_and(|procedure| {
                procedure.command == crate::flight::BasicFlightCommand::Land
                    && procedure.fence.model_revision == binding.model_revision
                    && procedure.sample.request.session.as_u64() == binding.session
                    && procedure.sample.request.generation.as_u64() == binding.generation
            })
    }

    pub(in crate::adapter) fn holds_landing_motion(
        &self,
        binding: crate::flight::VirtualStickBinding,
    ) -> bool {
        self.matches_landing_authority(binding)
            && self.procedure.as_ref().is_some_and(|p| !p.manual_released)
    }

    pub(in crate::adapter) fn wait_for_mode_release(
        &mut self,
        frame: &ScopedControlFrame,
        initial: Option<crate::flight::VirtualStickVelocity>,
        now: Duration,
    ) {
        let Some(procedure) = &mut self.procedure else {
            return;
        };
        procedure.manual_descent = initial.is_some_and(|input| input.is_descent_only());
        let active_until = frame
            .admitted_timing
            .and_then(|timing| timing.active_remaining)
            .map_or(Duration::MAX, |remaining| now.saturating_add(remaining));
        procedure.preparation = Some(LandingPreparation {
            until: now
                .saturating_add(HANDOVER_TIMEOUT)
                .min(procedure.completion_until)
                .min(active_until),
            started: false,
        });
    }

    pub(in crate::adapter) fn preparing_landing(&self) -> bool {
        self.procedure.as_ref().is_some_and(|procedure| {
            procedure.preparation.is_some() && !terminal(procedure.sample.phase)
        })
    }

    pub(in crate::adapter) fn preparation_written(&mut self, now: Duration) {
        self.poll(now);
        let Some(procedure) = &mut self.procedure else {
            return;
        };
        let Some(preparation) = &mut procedure.preparation else {
            return;
        };
        if procedure.sample.phase != FlightProcedurePhase::Queued || preparation.started {
            return;
        }
        // The first neutral/disable write starts this finite procedure inside the
        // original send window. Subsequent writes share its unrenewable budget.
        preparation.started = true;
        procedure.fence.send_until = preparation.until;
    }

    pub(in crate::adapter) fn prepared_landing(
        &mut self,
        model_revision: u64,
        now: Duration,
    ) -> Option<(FlightFence, BasicFlightPacket)> {
        self.poll(now);
        let procedure = self.procedure.as_ref()?;
        if !procedure.preparation.as_ref()?.started
            || !self.permits(&procedure.fence, model_revision, now)
        {
            return None;
        }
        let procedure = self.procedure.as_mut()?;
        procedure.preparation = None;
        Some((procedure.fence.clone(), procedure.packet))
    }
}

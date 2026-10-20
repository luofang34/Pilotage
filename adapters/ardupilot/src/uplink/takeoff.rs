//! A native takeoff is complete only when fresh FC observations establish it.

use std::time::{Duration, Instant};

use pilotage_adapter_api::FlightProcedureRequest;
use pilotage_mavlink::codec::MAV_CMD_NAV_TAKEOFF;
use pilotage_mavlink::{
    LinkState, OutboundCommand, SendFence, TakeoffWriteRequirement, WriteReceipt,
};
use pilotage_protocol::{
    CommandOutcome, FlightProcedurePhase as Phase, FlightProcedureReason as Reason,
    ScopedControlFrame,
};

use super::{ArduPilotUplink, FlightPhase, TAKEOFF_ALTITUDE_M};

mod observation;
#[cfg(test)]
mod tests;

const SEND_BUDGET: Duration = Duration::from_millis(250);
const COMPLETION_BUDGET: Duration = Duration::from_secs(30);
const REPORT_MAX_AGE: Duration = Duration::from_secs(3);
const TAKEOFF_MARGIN_M: f32 = 0.5;
const MAX_TARGET_ALTITUDE_M: f32 = 120.0;

/// One retained local procedure update, independent of FC heartbeat freshness.
#[derive(Debug, Clone)]
pub(crate) struct TakeoffStatus {
    pub(crate) request: FlightProcedureRequest,
    pub(crate) revision: u64,
    pub(crate) changed_at: Instant,
    pub(crate) phase: Phase,
    pub(crate) reason: Option<Reason>,
    pub(crate) fc_outcome: Option<CommandOutcome>,
}

#[derive(Debug)]
pub(super) struct TakeoffProcedure {
    status: TakeoffStatus,
    source_incarnation: pilotage_adapter_api::SourceIncarnation,
    source_epoch: u32,
    source_id: u64,
    start_down_m: f32,
    home_offset_m: f32,
    requested_at: Instant,
    baseline_time_boot_ms: u32,
    baseline_position_stamp: pilotage_adapter_api::MeasurementStamp,
    target_altitude_m: f32,
    send_before: Instant,
    complete_before: Instant,
    fence: SendFence,
    receipt: WriteReceipt,
    blocks_motion: bool,
}

impl TakeoffProcedure {
    fn active(&self) -> bool {
        matches!(
            self.status.phase,
            Phase::Queued | Phase::Written | Phase::InProgress
        )
    }

    fn transition(&mut self, phase: Phase, reason: Option<Reason>, now: Instant) {
        if self.status.phase == phase && self.status.reason == reason {
            return;
        }
        self.status.phase = phase;
        self.status.reason = reason;
        self.status.changed_at = now;
        self.status.revision = self.status.revision.wrapping_add(1);
        if !self.active() {
            self.fence.invalidate();
        }
    }

    fn acknowledge(&mut self, outcome: CommandOutcome, now: Instant) {
        if self.status.fc_outcome != Some(outcome) {
            self.status.fc_outcome = Some(outcome);
            self.status.changed_at = now;
            self.status.revision = self.status.revision.wrapping_add(1);
        }
    }
}

impl ArduPilotUplink {
    pub(crate) fn bind_flight_state(&mut self, state: std::sync::Arc<std::sync::Mutex<LinkState>>) {
        self.flight_state = Some(state);
    }

    pub(crate) fn takeoff_status_cached(&self) -> Option<&TakeoffStatus> {
        self.takeoff.as_ref().map(|procedure| &procedure.status)
    }

    pub(super) fn takeoff_blocks_motion(&self) -> bool {
        self.takeoff
            .as_ref()
            .is_some_and(|procedure| procedure.blocks_motion)
    }

    pub(super) fn takeoff_active(&self) -> bool {
        self.takeoff.as_ref().is_some_and(TakeoffProcedure::active)
    }

    pub(super) fn takeoff_may_have_written(&self) -> bool {
        self.takeoff.as_ref().is_some_and(|procedure| {
            procedure.blocks_motion && procedure.receipt.sent_at_cached().is_some()
        })
    }

    /// Invalidates queued launch work; the result describes local cancellation only.
    pub(crate) fn cancel_takeoff(&mut self, now: Instant) {
        self.interrupt_takeoff(now, Reason::AuthorityLost);
    }

    pub(super) fn interrupt_takeoff(&mut self, now: Instant, reason: Reason) {
        let Some(procedure) = self.takeoff.as_mut().filter(|procedure| procedure.active()) else {
            return;
        };
        procedure.transition(Phase::Cancelled, Some(reason), now);
        self.velocity.send_replace(None);
    }

    /// Starts one explicit action; queue admission does not assert an FC response.
    pub(crate) fn start_takeoff(
        &mut self,
        frame: &ScopedControlFrame,
        state: &LinkState,
        now: Instant,
    ) -> Result<(), &'static str> {
        self.observe_takeoff(state, now);
        if state.system_id != self.system_id || state.component_id != self.component_id {
            return Err("takeoff observations do not identify the command target");
        }
        if self.takeoff_blocks_motion() || self.phase != FlightPhase::Ground {
            return Err("takeoff requires a recovered ground state");
        }
        let (observation, start_down_m) = observation::start_observation(state, now)?;
        let target_altitude_m = observation.altitude_m + TAKEOFF_ALTITUDE_M;
        if !target_altitude_m.is_finite()
            || !(0.0..=MAX_TARGET_ALTITUDE_M).contains(&target_altitude_m)
            || target_altitude_m == 0.0
        {
            return Err("takeoff target is outside the home-relative altitude bounds");
        }
        let (send_before, complete_before) = deadlines(frame, now)?;
        let action_id = frame.action_ids.first().copied().unwrap_or_default();
        let baseline_position_stamp = state
            .kinematics
            .ok_or("takeoff needs an authorized position")?
            .stamp;
        let fence_state = self
            .flight_state
            .clone()
            .ok_or("takeoff needs a bound flight-state write guard")?;
        let requirement = TakeoffWriteRequirement::new(
            state,
            pilotage_mavlink::ardupilot::ArduCopterMode::Guided.custom_mode(),
            REPORT_MAX_AGE,
        );
        let fence = SendFence::default().for_takeoff(fence_state, requirement);
        let procedure = TakeoffProcedure {
            status: TakeoffStatus {
                request: FlightProcedureRequest {
                    session: frame.session,
                    scope: frame.scope.clone(),
                    generation: frame.generation,
                    sequence: frame.sequence,
                    action_id,
                },
                revision: self.takeoff_revision.wrapping_add(1),
                changed_at: now,
                phase: Phase::Queued,
                reason: None,
                fc_outcome: None,
            },
            source_incarnation: state.source_incarnation,
            source_epoch: state.source_epoch,
            source_id: state.source_id,
            start_down_m,
            home_offset_m: observation.altitude_m + start_down_m,
            requested_at: now,
            baseline_time_boot_ms: observation.time_boot_ms,
            baseline_position_stamp,
            target_altitude_m,
            send_before,
            complete_before,
            fence,
            receipt: WriteReceipt::default(),
            blocks_motion: true,
        };
        self.queue_takeoff(procedure)
    }

    fn queue_takeoff(&mut self, procedure: TakeoffProcedure) -> Result<(), &'static str> {
        let outbound = OutboundCommand {
            payload_write: None,
            receipt: Some(procedure.receipt.clone()),
            fence: Some(procedure.fence.clone()),
            send_before: Some(procedure.send_before),
            command: MAV_CMD_NAV_TAKEOFF,
            params: [
                0.0,
                0.0,
                0.0,
                f32::NAN,
                0.0,
                0.0,
                procedure.target_altitude_m,
            ],
            target_system: self.system_id,
            target_component: self.component_id,
        };
        if self.commands.try_send(outbound).is_err() {
            self.send_failures = self.send_failures.wrapping_add(1);
            return Err("the ArduPilot command lane refused takeoff");
        }
        self.takeoff_revision = procedure.status.revision;
        self.velocity.send_replace(None);
        self.takeoff = Some(procedure);
        Ok(())
    }
}

fn deadlines(frame: &ScopedControlFrame, now: Instant) -> Result<(Instant, Instant), &'static str> {
    let send = frame
        .admitted_timing
        .map_or(SEND_BUDGET, |timing| timing.send_remaining);
    let completion = frame
        .admitted_timing
        .and_then(|timing| timing.completion_timeout)
        .unwrap_or(COMPLETION_BUDGET);
    if send.is_zero() || completion.is_zero() {
        return Err("the takeoff command deadline has expired");
    }
    let send_before = now
        .checked_add(send.min(SEND_BUDGET))
        .ok_or("invalid send deadline")?;
    let complete_before = now
        .checked_add(completion.min(COMPLETION_BUDGET))
        .ok_or("invalid completion deadline")?;
    Ok((send_before.min(complete_before), complete_before))
}

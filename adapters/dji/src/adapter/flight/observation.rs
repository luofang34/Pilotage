use std::time::Duration;

use pilotage_adapter_api::{CommandOutcome, FlightProcedurePhase};

use super::FlightExecution;
use crate::flight::{BasicFlightCommand, Mini3FlightReport};

impl FlightExecution {
    pub(in crate::adapter) fn observe_flight(
        &mut self,
        report: Mini3FlightReport,
        model_revision: u64,
        now: Duration,
    ) {
        self.poll(now);
        let Some(procedure) = &mut self.procedure else {
            return;
        };
        if procedure.fence.model_revision != model_revision
            || procedure.sample.phase != FlightProcedurePhase::InProgress
            || procedure.sample.fc_outcome != Some(CommandOutcome::Accepted)
        {
            return;
        }
        if procedure.flight_sequence.is_some_and(|previous| {
            let delta = report.sequence.wrapping_sub(previous);
            delta == 0 || delta >= 0x8000
        }) {
            return;
        }
        procedure.flight_sequence = Some(report.sequence);
        if self.observe_landing_override(report, now) {
            return;
        }
        let Some(procedure) = &mut self.procedure else {
            return;
        };
        let complete = match procedure.command {
            BasicFlightCommand::Takeoff => report.is_flying && report.motors_running,
            BasicFlightCommand::Land => !report.is_flying && !report.motors_running,
            BasicFlightCommand::ReturnHome | BasicFlightCommand::CancelLanding => false,
        };
        if !complete {
            procedure.completion_reports = 0;
            procedure.completion_received = None;
            return;
        }
        // One buffered read is one observation, regardless of packet count.
        if procedure
            .completion_received
            .is_none_or(|received| now > received)
        {
            procedure.completion_reports = procedure.completion_reports.saturating_add(1);
            procedure.completion_received = Some(now);
        }
        if procedure.completion_reports >= 3 {
            // This establishes observed state, not which controller caused it.
            self.transition(FlightProcedurePhase::Completed, None, None, now);
        }
    }
}

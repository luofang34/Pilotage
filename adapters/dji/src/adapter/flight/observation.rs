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
            || procedure.command != BasicFlightCommand::Takeoff
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
        if !report.is_flying || !report.motors_running {
            procedure.airborne_reports = 0;
            procedure.airborne_received = None;
            return;
        }
        // One buffered read is one observation, regardless of packet count.
        if procedure
            .airborne_received
            .is_none_or(|received| now > received)
        {
            procedure.airborne_reports = procedure.airborne_reports.saturating_add(1);
            procedure.airborne_received = Some(now);
        }
        if procedure.airborne_reports >= 3 {
            // This establishes reported flight, not who controls the aircraft.
            self.transition(FlightProcedurePhase::Completed, None, None, now);
        }
    }
}

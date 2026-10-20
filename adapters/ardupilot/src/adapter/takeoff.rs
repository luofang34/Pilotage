use std::time::Instant;

use pilotage_adapter_api::{
    ActionResult, ApplyOutcome, Disposition, FlightProcedureSample, RejectReason, TelemetryBatch,
};
use pilotage_mavlink::sampling::report_only_sample;
use pilotage_protocol::{FlightProcedureKind, ScopedControlFrame};
use pilotage_timing::{MonoTimestamp, SimTick};

use super::ArduPilotAdapter;

impl ArduPilotAdapter {
    pub(super) fn apply_takeoff(
        &mut self,
        frame: &ScopedControlFrame,
        tick: SimTick,
    ) -> ApplyOutcome {
        let result = match (self.state.lock(), self.uplink.as_mut()) {
            (Ok(latest), Some(uplink)) => uplink.start_takeoff(frame, &latest, Instant::now()),
            _ => Err("the flight-control link is unavailable"),
        };
        match result {
            Ok(()) => ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results: frame
                    .actions
                    .iter()
                    .copied()
                    .map(ActionResult::accepted)
                    .collect(),
            },
            Err(detail) => ApplyOutcome {
                tick,
                disposition: Disposition::Rejected(RejectReason::Other(detail.to_owned())),
                action_results: frame
                    .actions
                    .iter()
                    .map(|action| ActionResult::rejected(*action, detail))
                    .collect(),
            },
        }
    }

    pub(super) fn attach_flight_procedure(&self, batch: &mut TelemetryBatch) {
        let Some(status) = self
            .uplink
            .as_ref()
            .and_then(crate::uplink::ArduPilotUplink::takeoff_status_cached)
        else {
            return;
        };
        let elapsed = status
            .changed_at
            .checked_duration_since(self.started_at)
            .unwrap_or_default();
        let report = FlightProcedureSample {
            request: status.request.clone(),
            adapter_incarnation: self.incarnation,
            revision: status.revision,
            changed_at: MonoTimestamp::from_nanos(
                u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
            ),
            kind: FlightProcedureKind::Takeoff,
            phase: status.phase,
            reason: status.reason,
            fc_outcome: status.fc_outcome,
        };
        if batch.samples.is_empty() {
            batch
                .samples
                .push(report_only_sample(self.vehicle, SimTick::new(0)));
        }
        for sample in &mut batch.samples {
            sample.flight_procedure = Some(report.clone());
        }
    }
}

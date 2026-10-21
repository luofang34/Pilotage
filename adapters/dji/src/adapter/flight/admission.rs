use std::time::Duration;

use pilotage_adapter_api::{
    ActionResult, ApplyOutcome, Disposition, FlightProcedureReason, RejectReason, TelemetryBatch,
};
use pilotage_protocol::{ControlAction, ScopeId, ScopedControlFrame, VehicleId};

use super::super::{
    DjiAdapter, DjiControlMode, FLIGHT_SCOPE, GIMBAL_SCOPE, input::Framing, observation,
};
use crate::flight::BasicFlightCommand;

impl DjiAdapter {
    pub(in crate::adapter) fn flight_enabled(&self) -> bool {
        self.experimental_flight
            && self.control.mode() == DjiControlMode::Observed
            && self.connection.input.framing() == Framing::LogicLink
    }

    pub(in crate::adapter) fn apply_flight(
        &mut self,
        frame: &ScopedControlFrame,
        now: Duration,
    ) -> ApplyOutcome {
        let result = self.admit_flight(frame, now);
        tracing::debug!(elapsed_ns = now.as_nanos(), request = ?frame.action_ids,
            outcome = ?result, "DJI discrete flight admission");
        match result {
            Ok(()) => ApplyOutcome {
                tick: self.tick(),
                disposition: Disposition::Accepted,
                action_results: frame
                    .actions
                    .iter()
                    .copied()
                    .map(ActionResult::accepted)
                    .collect(),
            },
            Err(reason) => ApplyOutcome {
                tick: self.tick(),
                action_results: frame
                    .actions
                    .iter()
                    .map(|action| ActionResult::rejected(*action, rejection_detail(&reason)))
                    .collect(),
                disposition: Disposition::Rejected(reason),
            },
        }
    }

    fn admit_flight(
        &mut self,
        frame: &ScopedControlFrame,
        now: Duration,
    ) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if !self.flight_enabled() || frame.scope.as_str() != FLIGHT_SCOPE {
            return Err(RejectReason::UnknownScope);
        }
        if self.connection.motion.link_loss {
            return Err(RejectReason::LinkLossEngaged);
        }
        let command = flight_demand(frame)?;
        if !self.connection.input_clear() || !self.connection.version_observed {
            return Err(super::reject(
                "the upper USB receive path and RC probe must be available",
            ));
        }
        self.connection.queue_flight(
            frame,
            command,
            self.control.sequence,
            &self.observations,
            now,
        )?;
        self.control.cancel_observed();
        self.control.sequence = self.control.sequence.wrapping_add(1);
        Ok(())
    }

    pub(in crate::adapter) fn attach_flight_progress(&self, batch: &mut TelemetryBatch) {
        let Some(progress) = self.connection.flight.progress_cached() else {
            return;
        };
        if batch.samples.is_empty() {
            batch
                .samples
                .push(observation::telemetry(self.vehicle, self.tick(), None));
        }
        if let Some(sample) = batch.samples.first_mut() {
            sample.flight_procedure = Some(progress);
        }
    }

    pub(in crate::adapter) fn invalidate_scope(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        now: Duration,
    ) {
        if vehicle != self.vehicle {
            return;
        }
        if (scope.as_str() == FLIGHT_SCOPE || scope.as_str() == GIMBAL_SCOPE)
            && !self.connection.mode_session.retained()
        {
            self.connection.mode_session.cancel();
        }
        if scope.as_str() == FLIGHT_SCOPE {
            self.connection.motion.stop(now);
            self.connection
                .invalidate_flight(FlightProcedureReason::AuthorityLost, now);
        } else if scope.as_str() == GIMBAL_SCOPE {
            self.control.cancel_observed();
            self.connection.cancel_movement();
        }
    }
}

fn flight_demand(frame: &ScopedControlFrame) -> Result<BasicFlightCommand, RejectReason> {
    if frame.carries_payload()
        || frame.intent.is_some()
        || frame.action_ids.len() != 1
        || frame.action_ids.first() == Some(&0)
    {
        return Err(super::reject(
            "only one correlated discrete DJI flight action is permitted",
        ));
    }
    match frame.actions.as_slice() {
        [ControlAction::Takeoff] => Ok(BasicFlightCommand::Takeoff),
        [ControlAction::Land] => Ok(BasicFlightCommand::Land),
        _ => Err(super::reject(
            "only experimental Takeoff and Land are implemented",
        )),
    }
}

fn rejection_detail(reason: &RejectReason) -> String {
    match reason {
        RejectReason::Other(detail) => detail.clone(),
        RejectReason::Fenced => {
            "this command's control lease or request is no longer valid".to_owned()
        }
        RejectReason::MeasurementUnavailable => {
            "fresh Mini 3 flight data is unavailable".to_owned()
        }
        _ => format!("{reason:?}"),
    }
}

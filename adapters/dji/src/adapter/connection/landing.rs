use std::time::Duration;

use pilotage_adapter_api::FlightProcedureReason;

use super::{Connection, Pending, Purpose};
use crate::{adapter::observation::Observations, flight::BasicFlightCommand};

impl Connection {
    pub(in crate::adapter) fn drive_landing(&mut self, observations: &Observations, now: Duration) {
        self.flight.poll(now);
        if !self.flight.preparing_landing() || self.pending.is_some() {
            return;
        }
        if self.fault.is_some() || !self.input_clear() || self.mode_session.active() {
            return;
        }
        let until = observations.flight_deadline(BasicFlightCommand::Land, now);
        if self.mode_session.unresolved() || until.is_none() {
            self.invalidate_flight(FlightProcedureReason::MeasurementUnavailable, now);
            return;
        }
        let Some((fence, packet)) = self
            .flight
            .prepared_landing(observations.model_revision(), now)
        else {
            return;
        };
        self.pending = Some(Pending {
            bytes: self.input.framing().command(packet.bytes()),
            offset: 0,
            expires_at: until.map(|until| until.min(fence.send_until)),
            purpose: Purpose::Flight(fence),
        });
    }
}

impl Connection {
    pub(in crate::adapter) fn landing_motion(
        &mut self,
        input: crate::adapter::motion::MotionInput,
        observations: &Observations,
        now: Duration,
        sequence: &mut u16,
    ) -> Result<(), pilotage_adapter_api::RejectReason> {
        if !self.input_clear() || self.fault.is_some() || observations.model_deadline(now).is_none()
        {
            return Err(pilotage_adapter_api::RejectReason::MeasurementUnavailable);
        }
        if self.pending.is_some()
            || self.mode_session.active()
            || self.mode_session.unresolved()
            || observations
                .flight_cached(now)
                .is_none_or(|report| !report.is_flying || !report.motors_running)
        {
            return Ok(());
        }
        let Some((fence, packet)) = self.flight.landing_override_input(input, now, *sequence)?
        else {
            return Ok(());
        };
        *sequence = sequence.wrapping_add(1);
        self.pending = Some(Pending {
            bytes: self.input.framing().command(packet.bytes()),
            offset: 0,
            expires_at: Some(fence.send_until),
            purpose: Purpose::LandingOverride(fence),
        });
        Ok(())
    }

    pub(super) fn prepare_landing_override(
        &mut self,
        fence: &super::super::flight::FlightFence,
        observations: &Observations,
        now: Duration,
    ) -> bool {
        let permitted = self.input_clear()
            && self.version_observed
            && observations.model_deadline(now).is_some()
            && self
                .flight
                .permits_landing_override(fence, observations.model_revision(), now);
        if !permitted {
            self.invalidate_flight(FlightProcedureReason::MeasurementUnavailable, now);
        }
        permitted
    }
}

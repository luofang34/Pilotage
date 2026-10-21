use std::time::Duration;

use pilotage_adapter_api::{FlightProcedureReason, RejectReason};
use pilotage_protocol::ScopedControlFrame;
use turret_core::CommandIdentity;

use super::super::{flight::FlightFence, observation::Observations};
use super::{Connection, DjiFault};
use crate::{GimbalPitchTarget, flight::BasicFlightCommand};

pub(super) struct Pending {
    pub(super) bytes: Vec<u8>,
    pub(super) offset: usize,
    pub(super) purpose: Purpose,
    pub(super) expires_at: Option<Duration>,
}

#[derive(Clone)]
pub(super) enum Purpose {
    Probe,
    Stick(crate::flight::VirtualStickWrite),
    Pitch {
        target: GimbalPitchTarget,
        identity: CommandIdentity,
    },
    Flight(FlightFence),
    Mode(super::super::virtual_stick::ModeFence),
}

impl Purpose {
    pub(super) fn pitch(&self) -> Option<(GimbalPitchTarget, CommandIdentity)> {
        match self {
            Self::Pitch { target, identity } => Some((*target, *identity)),
            _ => None,
        }
    }
}

impl Connection {
    pub(in crate::adapter) fn queue_flight(
        &mut self,
        frame: &ScopedControlFrame,
        command: BasicFlightCommand,
        sequence: u16,
        observations: &Observations,
        now: Duration,
    ) -> Result<(), RejectReason> {
        if self.mode_session.active()
            || (command == BasicFlightCommand::Takeoff && self.mode_session.unresolved())
        {
            return Err(RejectReason::Other(
                "virtual-stick diagnostic must finish with confirmed cleanup before Takeoff"
                    .to_owned(),
            ));
        }
        self.flight.validate(frame, command)?;
        let until = observations
            .flight_deadline(command, now)
            .ok_or_else(|| RejectReason::Other(match command {
                BasicFlightCommand::Takeoff => "Takeoff requires fresh Mini 3 stopped-motor data, not flying, and at least 25% battery",
                _ => "Land requires fresh confirmed Mini 3 flight data",
            }.to_owned()))?;
        super::super::flight::check_timing(frame)?;
        if !self.cancel_movement() {
            return Err(RejectReason::MeasurementUnavailable);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| matches!(pending.purpose, Purpose::Probe))
        {
            return Err(RejectReason::Other(
                "the RC probe has not been written".to_owned(),
            ));
        }
        if !self.invalidate_flight(FlightProcedureReason::Interrupted, now) {
            return Err(RejectReason::MeasurementUnavailable);
        }
        let packet = command.packet(sequence);
        let fence = self.flight.begin(
            frame,
            command,
            packet,
            observations.model_revision(),
            now,
            until,
        )?;
        self.pending = Some(Pending {
            bytes: self.input.framing().command(packet.bytes()),
            offset: 0,
            expires_at: Some(fence.send_until),
            purpose: Purpose::Flight(fence),
        });
        Ok(())
    }

    pub(in crate::adapter) fn invalidate_flight(
        &mut self,
        reason: FlightProcedureReason,
        now: Duration,
    ) -> bool {
        self.flight.invalidate(reason, now);
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| matches!(pending.purpose, Purpose::Flight(_)))
        {
            if self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.offset > 0)
            {
                self.fail(DjiFault::PartialCommand);
                return false;
            }
            self.pending = None;
        }
        true
    }
}

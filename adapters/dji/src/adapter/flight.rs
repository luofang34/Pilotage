//! Discrete flight requests retain their identity through the only writer.

mod admission;
mod observation;
mod progress;

use std::time::Duration;

use pilotage_adapter_api::{
    FlightProcedureKind, FlightProcedurePhase, FlightProcedureRequest, FlightProcedureSample,
    RejectReason, SourceIncarnation,
};
use pilotage_protocol::ScopedControlFrame;
use pilotage_timing::MonoTimestamp;

use crate::flight::{BasicFlightCommand, BasicFlightPacket};

const MAX_SEND_DELAY: Duration = Duration::from_millis(250);
const ACK_TIMEOUT: Duration = Duration::from_secs(1);
const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, PartialEq, Eq)]
pub(super) struct FlightFence {
    request: FlightProcedureRequest,
    incarnation: SourceIncarnation,
    model_revision: u64,
    pub(super) send_until: Duration,
}

struct Procedure {
    sample: FlightProcedureSample,
    fence: FlightFence,
    packet: BasicFlightPacket,
    command: BasicFlightCommand,
    completion_until: Duration,
    ack_until: Option<Duration>,
    written: bool,
    flight_sequence: Option<u16>,
    airborne_reports: u8,
    airborne_received: Option<Duration>,
}

pub(super) struct FlightExecution {
    incarnation: SourceIncarnation,
    authority: Option<(u64, u64)>,
    revoked: bool,
    procedure: Option<Procedure>,
    takeoff_uncertain: bool,
    now: Duration,
}

impl FlightExecution {
    pub(in crate::adapter) fn active(&self) -> bool {
        self.procedure
            .as_ref()
            .is_some_and(|procedure| !terminal(procedure.sample.phase))
    }

    pub(super) fn new(incarnation: SourceIncarnation) -> Self {
        Self {
            incarnation,
            authority: None,
            revoked: false,
            procedure: None,
            takeoff_uncertain: false,
            now: Duration::ZERO,
        }
    }

    pub(super) fn validate(
        &self,
        frame: &ScopedControlFrame,
        command: BasicFlightCommand,
    ) -> Result<(), RejectReason> {
        let incoming = (frame.session.as_u64(), frame.generation.as_u64());
        if let Some(current) = self.authority
            && ((incoming == current && self.revoked)
                || (incoming.0 == current.0
                    && incoming.1 != current.1
                    && incoming.1.wrapping_sub(current.1) >= 1 << 63))
        {
            return Err(RejectReason::Fenced);
        }
        if command == BasicFlightCommand::Takeoff && self.takeoff_uncertain {
            return Err(reject(
                "Takeoff outcome is unresolved; restart the attachment to retry",
            ));
        }
        if let Some(current) = &self.procedure {
            if current.sample.request == request(frame) {
                return Err(RejectReason::Fenced);
            }
            if !terminal(current.sample.phase)
                && !(command == BasicFlightCommand::Land
                    && current.command == BasicFlightCommand::Takeoff)
            {
                return Err(reject("a DJI flight procedure is already pending"));
            }
        }
        Ok(())
    }

    pub(super) fn begin(
        &mut self,
        frame: &ScopedControlFrame,
        command: BasicFlightCommand,
        packet: BasicFlightPacket,
        model_revision: u64,
        now: Duration,
        observed_until: Duration,
    ) -> Result<FlightFence, RejectReason> {
        self.validate(frame, command)?;
        let (remaining, completion) = check_timing(frame)?;
        let fence = FlightFence {
            request: request(frame),
            incarnation: self.incarnation,
            model_revision,
            send_until: now
                .saturating_add(remaining.min(completion))
                .min(observed_until),
        };
        self.authority = Some((frame.session.as_u64(), frame.generation.as_u64()));
        self.revoked = false;
        self.now = now;
        self.procedure = Some(Procedure {
            sample: FlightProcedureSample {
                request: fence.request.clone(),
                adapter_incarnation: self.incarnation,
                revision: 1,
                changed_at: timestamp(now),
                kind: if command == BasicFlightCommand::Takeoff {
                    FlightProcedureKind::Takeoff
                } else {
                    FlightProcedureKind::Land
                },
                phase: FlightProcedurePhase::Queued,
                reason: None,
                fc_outcome: None,
            },
            fence: fence.clone(),
            packet,
            command,
            completion_until: now.saturating_add(completion),
            ack_until: None,
            written: false,
            flight_sequence: None,
            airborne_reports: 0,
            airborne_received: None,
        });
        Ok(fence)
    }

    pub(super) fn permits(&self, fence: &FlightFence, model_revision: u64, now: Duration) -> bool {
        !self.revoked
            && fence.incarnation == self.incarnation
            && fence.model_revision == model_revision
            && now < fence.send_until
            && self.procedure.as_ref().is_some_and(|procedure| {
                procedure.fence == *fence
                    && procedure.sample.phase == FlightProcedurePhase::Queued
                    && self.authority
                        == Some((
                            fence.request.session.as_u64(),
                            fence.request.generation.as_u64(),
                        ))
            })
    }

    pub(super) fn command_cached(&self) -> Option<BasicFlightCommand> {
        self.procedure.as_ref().map(|procedure| procedure.command)
    }

    pub(super) fn progress_cached(&self) -> Option<FlightProcedureSample> {
        self.procedure
            .as_ref()
            .map(|procedure| procedure.sample.clone())
    }
}

fn request(frame: &ScopedControlFrame) -> FlightProcedureRequest {
    FlightProcedureRequest {
        session: frame.session,
        scope: frame.scope.clone(),
        generation: frame.generation,
        sequence: frame.sequence,
        action_id: frame.action_ids.first().copied().unwrap_or(0),
    }
}

fn reject(detail: &str) -> RejectReason {
    RejectReason::Other(detail.to_owned())
}

pub(super) fn check_timing(
    frame: &ScopedControlFrame,
) -> Result<(Duration, Duration), RejectReason> {
    let timing = frame
        .admitted_timing
        .ok_or_else(|| reject("admitted timing is absent"))?;
    if !timing.window_bounded {
        return Err(reject("a live executor validity window is required"));
    }
    let remaining = timing.send_remaining.min(MAX_SEND_DELAY);
    let remaining = timing
        .active_remaining
        .map_or(remaining, |active| remaining.min(active));
    let completion = timing
        .completion_timeout
        .unwrap_or(OBSERVATION_TIMEOUT)
        .min(OBSERVATION_TIMEOUT);
    if remaining.is_zero() || completion.is_zero() {
        return Err(reject("the flight request has expired"));
    }
    Ok((remaining, completion))
}

fn timestamp(now: Duration) -> MonoTimestamp {
    MonoTimestamp::from_nanos(now.as_nanos().min(u128::from(u64::MAX)) as u64)
}

fn terminal(phase: FlightProcedurePhase) -> bool {
    matches!(
        phase,
        FlightProcedurePhase::Completed
            | FlightProcedurePhase::Failed
            | FlightProcedurePhase::TimedOut
            | FlightProcedurePhase::Cancelled
    )
}

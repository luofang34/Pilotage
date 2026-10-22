//! Discrete flight requests retain their identity through the only writer.

mod admission;
mod confirmation;
mod cycle;
mod observation;
mod override_control;
mod preparation;
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
    completion_reports: u8,
    completion_received: Option<Duration>,
    preparation: Option<preparation::LandingPreparation>,
    manual_sequence: u32,
    manual_ready: bool,
    manual_descent: bool,
    manual_released: bool,
    manual_resumed: bool,
    override_control: Option<override_control::LandingOverride>,
}

pub(super) struct FlightExecution {
    incarnation: SourceIncarnation,
    authority: Option<(u64, u64)>,
    revoked: bool,
    authority_loss: Option<(u64, u64)>,
    procedure: Option<Procedure>,
    takeoff_uncertain: bool,
    now: Duration,
    confirmation: confirmation::Confirmation,
    cycle: cycle::MotorCycle,
}

impl FlightExecution {
    pub(in crate::adapter) fn retain_authority(&mut self, session: u64, generation: u64) {
        self.authority = Some((session, generation));
        self.revoked = false;
    }

    pub(in crate::adapter) fn holds_authority(&self) -> bool {
        self.authority.is_some() && !self.revoked
    }

    pub(in crate::adapter) fn revoke_motion_authority(
        &mut self,
        binding: crate::flight::VirtualStickBinding,
    ) {
        if self.authority != Some((binding.session, binding.generation)) {
            self.revoked = false;
        }
        self.authority = Some((binding.session, binding.generation));
        self.revoke_device_authority();
    }

    pub(in crate::adapter) fn revoke_device_authority(&mut self) {
        if !self.revoked {
            self.authority_loss = self.authority;
        }
        self.revoked = true;
    }

    pub(in crate::adapter) fn take_authority_loss(&mut self) -> Option<(u64, u64)> {
        self.authority_loss.take()
    }

    pub(in crate::adapter) fn authority_valid(&self, session: u64, generation: u64) -> bool {
        self.authority.is_none_or(|current| {
            !((current == (session, generation) && self.revoked)
                || (current.0 == session
                    && current.1 != generation
                    && generation.wrapping_sub(current.1) >= 1 << 63))
        })
    }

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
            authority_loss: None,
            procedure: None,
            takeoff_uncertain: false,
            now: Duration::ZERO,
            confirmation: Default::default(),
            cycle: Default::default(),
        }
    }

    pub(super) fn validate(
        &self,
        frame: &ScopedControlFrame,
        command: BasicFlightCommand,
    ) -> Result<(), RejectReason> {
        if !self.authority_valid(frame.session.as_u64(), frame.generation.as_u64()) {
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
        self.confirmation = Default::default();
        if command == BasicFlightCommand::Takeoff {
            self.cycle = cycle::MotorCycle::new(model_revision);
        }
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
            completion_reports: 0,
            completion_received: None,
            preparation: None,
            manual_sequence: frame.sequence.as_u32(),
            manual_ready: false,
            manual_descent: false,
            manual_released: false,
            manual_resumed: false,
            override_control: None,
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
    phase.is_terminal()
}

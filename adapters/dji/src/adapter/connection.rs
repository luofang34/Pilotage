use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::error;

use super::{DjiFault, observation::Observations, recording::record_blocking};
use crate::{GimbalPitchTarget, RawSerial, RouteAddress};

use super::execution::Execution;
use super::input::{Framing, Input};
use turret_core::{CommandIdentity, PayloadCapabilities};
use turret_dji::Mini3ControlProfile;

mod keyframe;
mod landing;
mod motion;
mod pending;
mod virtual_stick;
mod writer;

use super::flight::FlightExecution;
use pending::{Pending, Purpose};

const READ_BUDGET: usize = 64 * 1024;

pub(super) trait Transport: Send {
    fn framing(&self) -> Framing {
        Framing::Duml
    }
    fn close_blocking(self: Box<Self>) -> io::Result<()> {
        Ok(())
    }
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize>;
    fn write(&mut self, bytes: &[u8], send_before: Instant) -> io::Result<usize>;
}

impl Transport for RawSerial {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.try_read(bytes)
            .map_err(|source| io::Error::new(source.kind(), source))
    }

    fn write(&mut self, bytes: &[u8], send_before: Instant) -> io::Result<usize> {
        if Instant::now() >= send_before {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.try_write(bytes)
            .map_err(|source| io::Error::new(source.kind(), source))
    }
}

pub(super) struct Connection {
    origin: Instant,
    pub(super) execution: Execution,
    pub(super) flight: FlightExecution,
    pub(super) motion: super::motion::Motion,
    pub(super) mode_session: super::virtual_stick::ModeSession,
    profile: Mini3ControlProfile,
    transport: Option<Box<dyn Transport>>,
    pub(super) cleanup_confirmed: bool,
    capture: Box<dyn Write + Send>,
    capture_path: PathBuf,
    pub(super) input: Input,
    pub(super) keyframes: keyframe::Keyframes,
    pending: Option<Pending>,
    written_pitch: Option<(GimbalPitchTarget, CommandIdentity)>,
    drained: bool,
    pub(super) fault: Option<DjiFault>,
    pub(super) version_observed: bool,
    pub(super) received_bytes: u64,
    pub(super) written_bytes: u64,
}

impl Connection {
    pub(super) fn new(
        transport: Box<dyn Transport>,
        capture: Box<dyn Write + Send>,
        capture_path: PathBuf,
        profile: Mini3ControlProfile,
        capabilities: PayloadCapabilities,
        incarnation: pilotage_adapter_api::SourceIncarnation,
        origin: Instant,
    ) -> Self {
        let framing = transport.framing();
        Self {
            origin,
            execution: Execution::new(capabilities, 0),
            flight: FlightExecution::new(incarnation),
            motion: super::motion::Motion::default(),
            mode_session: super::virtual_stick::ModeSession::new(incarnation),
            profile,
            transport: Some(transport),
            cleanup_confirmed: false,
            capture,
            capture_path,
            input: Input::new(framing),
            keyframes: keyframe::Keyframes::new(incarnation.into_bytes()),
            pending: Some(Pending {
                bytes: framing.command(&framing.rc_version_request()),
                offset: 0,
                purpose: Purpose::Probe,
                expires_at: None,
            }),
            written_pitch: None,
            drained: false,
            fault: None,
            version_observed: false,
            received_bytes: 0,
            written_bytes: 0,
        }
    }

    pub(super) fn input_clear(&self) -> bool {
        self.fault.is_none() && self.drained && self.input.command_boundary()
    }

    pub(super) fn pending_bytes(&self) -> usize {
        self.pending
            .as_ref()
            .map_or(0, |pending| pending.bytes.len() - pending.offset)
    }

    pub(super) fn queue_pitch_until(
        &mut self,
        pitch: GimbalPitchTarget,
        sequence: u16,
        now: Duration,
        expires_at: Duration,
    ) -> bool {
        if self.pending.is_some()
            || self.keyframes.active.is_some()
            || self.fault.is_some()
            || (self.mode_session.active() && !self.mode_session.enabled())
        {
            return false;
        }
        let command = match self.profile.encode_pitch(pitch, sequence) {
            Ok(command) => command,
            Err(source) => {
                self.fail(DjiFault::Profile {
                    operation: "encode pitch",
                    source,
                });
                return false;
            }
        };
        let identity = match self.execution.admit(pitch, sequence, now, expires_at) {
            Ok(identity) => identity,
            Err(error) => {
                tracing::debug!(%error, "DJI target rejected by payload executor");
                return false;
            }
        };
        self.pending = Some(Pending {
            bytes: self.input.framing().command(&command),
            offset: 0,
            purpose: Purpose::Pitch {
                target: pitch,
                identity,
            },
            expires_at: Some(expires_at),
        });
        true
    }

    pub(super) fn pending_pitch(&self) -> Option<GimbalPitchTarget> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.purpose.pitch().map(|(target, _)| target))
    }

    pub(super) fn pending_pitch_identity(&self) -> Option<CommandIdentity> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.purpose.pitch().map(|(_, identity)| identity))
    }

    pub(super) fn cancel_movement(&mut self) -> bool {
        if let Err(fault) = self.keyframes.preempt_for_movement() {
            self.fail(fault);
            return false;
        }
        if let Some(pending) = &self.pending {
            if pending.purpose.pitch().is_some() && pending.offset > 0 {
                self.fail(DjiFault::PartialCommand);
                return false;
            }
            if pending.purpose.pitch().is_some() {
                self.pending = None;
            }
        }
        self.execution.cancel();
        true
    }

    pub(super) fn invalidate_model(&mut self) {
        self.motion.lost();
        self.mode_session.source_changed();
        self.cancel_mode_pending();
        self.cancel_movement();
        self.keyframes.close();
    }

    pub(super) fn read_available_blocking(
        &mut self,
        now: Duration,
        observations: &mut Observations,
    ) {
        self.flight.poll(now);
        self.drained = false;
        let mut read = 0;
        let mut buffer = [0; 4096];
        while read < READ_BUDGET {
            let Some(transport) = &mut self.transport else {
                return;
            };
            let capacity = buffer.len().min(READ_BUDGET - read);
            match transport.read(&mut buffer[..capacity]) {
                Ok(0) => {
                    self.fail(DjiFault::Transport {
                        operation: "read",
                        source: io::ErrorKind::UnexpectedEof.into(),
                    });
                    return;
                }
                Ok(count) => {
                    read = read.wrapping_add(count);
                    self.received_bytes = self.received_bytes.wrapping_add(count as u64);
                    if !self.record_blocking(now, "rx", &buffer[..count]) {
                        return;
                    }
                    self.parse(&buffer[..count], now, observations);
                    if self.fault.is_some() {
                        return;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.drained = true;
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    // The unread tail may still belong to the same queued batch.
                    observations.invalidate_continuity();
                    return;
                }
                Err(source) => {
                    self.fail(DjiFault::Transport {
                        operation: "read",
                        source,
                    });
                    return;
                }
            }
        }
        // A backlog split by the work budget cannot stand in for new arrivals.
        observations.invalidate_continuity();
    }

    fn parse(&mut self, bytes: &[u8], now: Duration, observations: &mut Observations) {
        let frames = match self.input.receive(bytes, now) {
            Ok(frames) => frames,
            Err(fault) => {
                self.fail(fault);
                return;
            }
        };
        for frame in frames {
            self.execution.response(&frame, now);
            self.flight.response(&frame, now);
            if frame.sender() == RouteAddress(6)
                && frame.receiver() == RouteAddress(self.input.framing().probe_sender())
                && frame.sequence() == 1
                && frame.flags() == 0x80
                && frame.command_set() == 0
                && frame.command_id() == 1
            {
                self.version_observed = true;
            }
            observations.receive(&frame, now);
            let binding = observations
                .model_deadline(now)
                .map(|_| observations.model_revision());
            self.mode_session.poll(now, binding);
            self.mode_session.receive(&frame, now, binding);
        }
    }

    pub(super) fn take_written_pitch(&mut self) -> Option<(GimbalPitchTarget, CommandIdentity)> {
        self.written_pitch.take()
    }

    fn record_blocking(&mut self, now: Duration, direction: &str, bytes: &[u8]) -> bool {
        match record_blocking(&mut self.capture, now, direction, bytes) {
            Ok(()) => true,
            Err(source) => {
                self.fail(DjiFault::Recording {
                    path: self.capture_path.clone(),
                    source,
                });
                false
            }
        }
    }

    pub(super) fn fail(&mut self, fault: DjiFault) {
        error!(%fault, "DJI transport failed; control is fenced");
        if let Some(pending) = &self.pending {
            if pending.offset > 0 {
                if let Some((_, identity)) = pending.purpose.pitch() {
                    self.execution.failed(identity);
                }
            } else {
                self.execution.cancel();
            }
        } else {
            self.execution.cancel();
        }
        self.motion.lost();
        self.flight.transport_failed();
        self.mode_session.transport_failed();
        self.fault = Some(fault);
        self.pending = None;
        // The fault fences all writes immediately. The owner closes off the control loop.
        self.input.close();
        self.keyframes.close();
    }
}

impl Connection {
    pub(super) fn close_transport_blocking(&mut self) {
        if let Some(transport) = self.transport.take() {
            match transport.close_blocking() {
                Ok(()) => self.cleanup_confirmed = true,
                Err(error) => {
                    tracing::error!(%error, "DJI attachment cleanup unconfirmed; do not reopen automatically")
                }
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.keyframes.close();
        self.close_transport_blocking();
    }
}

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

struct Pending {
    bytes: Vec<u8>,
    offset: usize,
    pitch: Option<GimbalPitchTarget>,
    expires_at: Option<Duration>,
    identity: Option<CommandIdentity>,
}

pub(super) struct Connection {
    origin: Instant,
    pub(super) execution: Execution,
    profile: Mini3ControlProfile,
    transport: Option<Box<dyn Transport>>,
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
            profile,
            transport: Some(transport),
            capture,
            capture_path,
            input: Input::new(framing),
            keyframes: keyframe::Keyframes::new(incarnation.into_bytes()),
            pending: Some(Pending {
                bytes: framing.command(&turret_dji::rc_version_request()),
                offset: 0,
                pitch: None,
                expires_at: None,
                identity: None,
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
        if self.pending.is_some() || self.keyframes.active.is_some() || self.fault.is_some() {
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
            pitch: Some(pitch),
            expires_at: Some(expires_at),
            identity: Some(identity),
        });
        true
    }

    pub(super) fn pending_pitch(&self) -> Option<GimbalPitchTarget> {
        self.pending.as_ref().and_then(|pending| pending.pitch)
    }

    pub(super) fn pending_pitch_identity(&self) -> Option<CommandIdentity> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.pitch.and(pending.identity))
    }

    pub(super) fn cancel_movement(&mut self) -> bool {
        if let Err(fault) = self.keyframes.preempt_for_movement() {
            self.fail(fault);
            return false;
        }
        if let Some(pending) = &self.pending {
            if pending.pitch.is_some() && pending.offset > 0 {
                self.fail(DjiFault::PartialCommand);
                return false;
            }
            if pending.pitch.is_some() {
                self.pending = None;
            }
        }
        self.execution.cancel();
        true
    }

    pub(super) fn invalidate_model(&mut self) {
        self.cancel_movement();
        self.keyframes.close();
    }

    pub(super) fn read_available_blocking(
        &mut self,
        now: Duration,
        observations: &mut Observations,
    ) {
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
            if frame.sender() == RouteAddress(6)
                && frame.receiver() == RouteAddress(10)
                && frame.sequence() == 1
                && frame.flags() == 0x80
                && frame.command_set() == 0
                && frame.command_id() == 1
            {
                self.version_observed = true;
            }
            observations.receive(&frame, now);
        }
    }

    pub(super) fn write_pending_blocking(
        &mut self,
        now: Duration,
        movement_ready: bool,
        completed_at: &mut impl FnMut() -> Duration,
    ) {
        let Some(pending) = &self.pending else { return };
        if pending.pitch.is_some() {
            if !movement_ready {
                self.cancel_movement();
                return;
            }
            if pending.offset > 0 && pending.expires_at.is_some_and(|until| now >= until) {
                self.fail(DjiFault::PartialCommand);
                return;
            }
            if let Err(error) = self.execution.prepare(now) {
                tracing::debug!(%error, "DJI queued target expired before write");
                self.pending = None;
                return;
            }
        }
        let Some(pending) = &self.pending else { return };
        let until = pending
            .expires_at
            .unwrap_or_else(|| now.saturating_add(Duration::from_millis(50)));
        let Some(send_before) = self.origin.checked_add(until) else {
            self.fail(DjiFault::Transport {
                operation: "compute write deadline",
                source: io::ErrorKind::InvalidInput.into(),
            });
            return;
        };
        let Some(transport) = &mut self.transport else {
            return;
        };
        let result = transport.write(&pending.bytes[pending.offset..], send_before);
        match result {
            Ok(0) => self.fail(DjiFault::Transport {
                operation: "write",
                source: io::ErrorKind::WriteZero.into(),
            }),
            Ok(count) => self.record_written_blocking(completed_at(), count),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(source) => {
                // A failed write may have reached the device even without a receipt.
                if let Some(identity) = self.pending.take().and_then(|pending| pending.identity) {
                    self.execution.failed(identity);
                }
                self.fail(DjiFault::Transport {
                    operation: "write",
                    source,
                });
            }
        }
    }

    fn record_written_blocking(&mut self, now: Duration, count: usize) {
        let Some(pending) = &mut self.pending else {
            return;
        };
        let actual = pending.bytes[pending.offset..pending.offset + count].to_vec();
        pending.offset = pending.offset.wrapping_add(count);
        let complete = pending.offset == pending.bytes.len();
        let pitch = pending.pitch;
        let identity = pending.identity;
        self.written_bytes = self.written_bytes.wrapping_add(count as u64);
        if complete {
            self.pending = None;
            if let Some(identity) = identity
                && let Err(source) = self.execution.sent(identity, now)
            {
                self.fail(DjiFault::Execution { source });
                return;
            }
        }
        if !self.record_blocking(now, "tx", &actual) {
            return;
        }
        if complete {
            self.pending = None;
            self.written_pitch = pitch.zip(identity);
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
        error!(%fault, "DJI connection closed; explicit restart required");
        if let Some(pending) = &self.pending {
            if pending.offset > 0 {
                if let Some(identity) = pending.identity {
                    self.execution.failed(identity);
                }
            } else {
                self.execution.cancel();
            }
        } else {
            self.execution.cancel();
        }
        self.fault = Some(fault);
        self.pending = None;
        // Closing prevents a partial packet from being followed by a new command.
        self.close_transport_blocking();
        self.input.close();
        self.keyframes.close();
    }
}

impl Connection {
    fn close_transport_blocking(&mut self) {
        if let Some(transport) = self.transport.take()
            && let Err(error) = transport.close_blocking()
        {
            tracing::error!(%error, "DJI attachment cleanup unconfirmed; do not reopen automatically");
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.keyframes.close();
        self.close_transport_blocking();
    }
}

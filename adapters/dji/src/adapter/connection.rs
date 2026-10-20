use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use tracing::error;

use super::{DjiFault, observation::Observations, recording::record_blocking};
use crate::{DumlParser, GimbalPitchTarget, RawSerial, RouteAddress, checksum};

const READ_BUDGET: usize = 64 * 1024;

pub(super) trait Transport: Send {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize>;
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize>;
}

impl Transport for RawSerial {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.try_read(bytes)
            .map_err(|source| io::Error::new(source.kind(), source))
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.try_write(bytes)
            .map_err(|source| io::Error::new(source.kind(), source))
    }
}

struct Pending {
    bytes: Vec<u8>,
    offset: usize,
    pitch: Option<GimbalPitchTarget>,
    expires_at: Option<Duration>,
}

pub(super) struct Connection {
    transport: Option<Box<dyn Transport>>,
    capture: Box<dyn Write + Send>,
    capture_path: PathBuf,
    parser: DumlParser,
    pending: Option<Pending>,
    written_pitch: Option<GimbalPitchTarget>,
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
    ) -> Self {
        Self {
            transport: Some(transport),
            capture,
            capture_path,
            parser: DumlParser::new(),
            pending: Some(Pending {
                bytes: version_request(),
                offset: 0,
                pitch: None,
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
        self.fault.is_none() && self.drained && self.parser.buffered_bytes() == 0
    }

    pub(super) fn pending_bytes(&self) -> usize {
        self.pending
            .as_ref()
            .map_or(0, |pending| pending.bytes.len() - pending.offset)
    }

    pub(super) fn queue_pitch(&mut self, pitch: GimbalPitchTarget, sequence: u16) -> bool {
        self.queue_pitch_until(pitch, sequence, None)
    }

    pub(super) fn queue_pitch_until(
        &mut self,
        pitch: GimbalPitchTarget,
        sequence: u16,
        expires_at: Option<Duration>,
    ) -> bool {
        if self.pending.is_some() || self.fault.is_some() {
            return false;
        }
        self.pending = Some(Pending {
            bytes: pitch.encode_app_command(sequence).to_vec(),
            offset: 0,
            pitch: Some(pitch),
            expires_at,
        });
        true
    }

    pub(super) fn pending_pitch(&self) -> Option<GimbalPitchTarget> {
        self.pending.as_ref().and_then(|pending| pending.pitch)
    }

    pub(super) fn cancel_movement(&mut self) -> bool {
        if let Some(pending) = &self.pending {
            if pending.pitch.is_some() && pending.offset > 0 {
                self.fail(DjiFault::PartialCommand);
                return false;
            }
            if pending.pitch.is_some() {
                self.pending = None;
            }
        }
        true
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
                    self.fail(DjiFault::Serial {
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
                    self.fail(DjiFault::Serial {
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
        let before = self.parser.stats();
        let mut frames = Vec::new();
        self.parser.push(bytes, |frame| frames.push(frame));
        if before.discarded_bytes != self.parser.stats().discarded_bytes
            || frames.iter().any(|frame| frame.version() != 1)
        {
            self.fail(DjiFault::Framing);
            return;
        }
        for frame in frames {
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

    pub(super) fn write_pending_blocking(&mut self, now: Duration, movement_ready: bool) {
        let Some(pending) = &self.pending else { return };
        if pending.pitch.is_some()
            && (!movement_ready || pending.expires_at.is_some_and(|until| now >= until))
        {
            self.cancel_movement();
            return;
        }
        let Some(transport) = &mut self.transport else {
            return;
        };
        let result = transport.write(&pending.bytes[pending.offset..]);
        match result {
            Ok(0) => self.fail(DjiFault::Serial {
                operation: "write",
                source: io::ErrorKind::WriteZero.into(),
            }),
            Ok(count) => self.record_written_blocking(now, count),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(source) => self.fail(DjiFault::Serial {
                operation: "write",
                source,
            }),
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
        self.written_bytes = self.written_bytes.wrapping_add(count as u64);
        if !self.record_blocking(now, "tx", &actual) {
            return;
        }
        if complete {
            self.pending = None;
            self.written_pitch = pitch;
        }
    }

    pub(super) fn take_written_pitch(&mut self) -> Option<GimbalPitchTarget> {
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

    fn fail(&mut self, fault: DjiFault) {
        error!(%fault, "DJI connection closed; explicit restart required");
        self.fault = Some(fault);
        self.pending = None;
        // Closing prevents a partial packet from being followed by a new command.
        self.transport = None;
    }
}

fn version_request() -> Vec<u8> {
    let mut bytes = vec![0x55, 13, 4, 0, 10, 6, 1, 0, 0x40, 0, 1];
    bytes[3] = checksum::header(&bytes[..3]);
    bytes.extend(checksum::frame(&bytes).to_le_bytes());
    bytes
}

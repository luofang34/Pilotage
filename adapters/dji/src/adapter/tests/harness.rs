use super::super::GIMBAL_SCOPE;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use pilotage_adapter_api::{
    ApplyOutcome, LinkLossEnactError, LinkLossPolicy, SourceIncarnation, TelemetryBatch,
};
use pilotage_protocol::{
    ControlIntent, ControlPayload, Generation, GimbalRateIntent, ScopeId, ScopedControlFrame,
    SequenceNum, SessionId, VehicleId,
};
use pilotage_timing::MonoTimestamp;

use super::super::{DjiAdapter, connection::Transport, input::Framing};
use crate::{DumlFrame, DumlParser};

#[derive(Default)]
struct Wire {
    input: VecDeque<u8>,
    output: Vec<u8>,
    deadlines: Vec<std::time::Instant>,
    capture: Vec<u8>,
    write_limit: usize,
    read_limit: usize,
    reads_before_interrupt: Option<usize>,
    disconnected: bool,
    capture_failed: bool,
    write_failed_after_accept: bool,
}

#[derive(Clone)]
struct Recording(Arc<Mutex<Wire>>, Framing);

impl Recording {
    fn lock(&self) -> io::Result<MutexGuard<'_, Wire>> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("test recording lock poisoned"))
    }
}

impl Transport for Recording {
    fn framing(&self) -> Framing {
        self.1
    }

    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut wire = self.lock()?;
        if wire.disconnected {
            return Ok(0);
        }
        if let Some(remaining) = wire.reads_before_interrupt {
            if remaining == 0 {
                wire.reads_before_interrupt = None;
                return Err(io::ErrorKind::Interrupted.into());
            }
            wire.reads_before_interrupt = Some(remaining - 1);
        }
        if wire.input.is_empty() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(wire.input.len()).min(wire.read_limit);
        for (destination, source) in bytes.iter_mut().zip(wire.input.drain(..count)) {
            *destination = source;
        }
        Ok(count)
    }

    fn write(&mut self, bytes: &[u8], send_before: std::time::Instant) -> io::Result<usize> {
        let mut wire = self.lock()?;
        if wire.write_limit == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = wire.write_limit.min(bytes.len());
        wire.deadlines.push(send_before);
        wire.output.extend(&bytes[..count]);
        if wire.write_failed_after_accept {
            return Err(io::ErrorKind::TimedOut.into());
        }
        Ok(count)
    }
}

impl Write for Recording {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut wire = self.lock()?;
        if wire.capture_failed {
            return Err(io::Error::other("test capture refused the write"));
        }
        wire.capture.extend(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) struct Harness {
    pub(super) adapter: DjiAdapter,
    recording: Recording,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self::with_mode(super::super::DjiControlMode::Observed)
    }

    pub(super) fn bench() -> Self {
        Self::with_mode(super::super::DjiControlMode::BenchInitialHorizontal)
    }

    pub(super) fn mobile() -> Self {
        Self::with_framing(super::super::DjiControlMode::Observed, Framing::LogicLink)
    }

    pub(super) fn flight(mode: super::super::DjiControlMode, framing: Framing) -> Self {
        Self::with_options(mode, framing, true)
    }

    fn with_mode(control_mode: super::super::DjiControlMode) -> Self {
        Self::with_framing(control_mode, Framing::Duml)
    }

    fn with_framing(control_mode: super::super::DjiControlMode, framing: Framing) -> Self {
        Self::with_options(control_mode, framing, false)
    }

    fn with_options(
        control_mode: super::super::DjiControlMode,
        framing: Framing,
        experimental_flight: bool,
    ) -> Self {
        let recording = Recording(
            Arc::new(Mutex::new(Wire {
                write_limit: usize::MAX,
                read_limit: usize::MAX,
                ..Wire::default()
            })),
            framing,
        );
        let adapter = DjiAdapter::from_connection(
            VehicleId::new(1),
            Box::new(recording.clone()),
            Box::new(recording.clone()),
            "injected-recording".into(),
            SourceIncarnation::new([42; 16]),
            control_mode,
            experimental_flight,
        )
        .unwrap();
        Self { adapter, recording }
    }

    pub(super) fn prime(&mut self) -> io::Result<()> {
        self.feed_model(87, 0)?;
        self.step(0);
        self.telemetry(0);
        for (index, pose) in observed_poses().into_iter().enumerate() {
            self.feed(&pose)?;
            self.step(index as u64 * 10);
        }
        Ok(())
    }

    pub(super) fn feed_model(&mut self, fc_type: u8, sequence: u16) -> io::Result<()> {
        let mut bytes = captured_hex_frames(include_str!(
            "../airframe/fixtures/mini3-reported-attitude.hex"
        ))
        .remove(0);
        bytes[6..8].copy_from_slice(&sequence.to_le_bytes());
        bytes[59] = fc_type;
        seal_duml(&mut bytes);
        let packet = packets(&bytes).remove(0);
        self.feed(&self.recording.1.command(packet.raw()))
    }

    pub(super) fn select_profile(&mut self, profile: turret_dji::Mini3ControlProfile) {
        let incarnation = SourceIncarnation::new([42; 16]);
        let mode = self.adapter.control.mode();
        let mut contract = profile.capabilities().unwrap();
        super::super::capability::set_availability(
            &mut contract,
            Some(turret_core::UnavailableReason::MissingFeedback),
        );
        self.adapter.profile = profile;
        self.adapter.connection = super::super::connection::Connection::new(
            Box::new(self.recording.clone()),
            Box::new(self.recording.clone()),
            "injected-recording".into(),
            profile,
            contract,
            incarnation,
            self.adapter.origin,
        );
        self.adapter.observations =
            super::super::observation::Observations::new(incarnation, profile);
        self.adapter.control = super::super::control::ControlState::new(mode, profile);
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.recording.lock()?.input.extend(bytes);
        Ok(())
    }

    pub(super) fn step(&mut self, millis: u64) {
        self.adapter
            .step_with_time_blocking(|| Duration::from_millis(millis));
    }

    pub(super) fn apply(&mut self, frame: ScopedControlFrame, millis: u64) -> ApplyOutcome {
        self.adapter.apply_at(&frame, Duration::from_millis(millis))
    }

    pub(super) fn telemetry(&mut self, millis: u64) -> TelemetryBatch {
        self.adapter.observations.sample(
            self.adapter.vehicle,
            self.adapter.tick(),
            Duration::from_millis(millis),
            self.adapter.connection.fault.is_none(),
        )
    }

    pub(super) fn link_loss(
        &mut self,
        policy: Option<LinkLossPolicy>,
        millis: u64,
    ) -> Result<(), LinkLossEnactError> {
        self.adapter.link_loss_with_time_blocking(
            VehicleId::new(1),
            &ScopeId::new(GIMBAL_SCOPE),
            policy,
            || Duration::from_millis(millis),
        )
    }

    pub(super) fn deadlines(&self) -> io::Result<Vec<std::time::Instant>> {
        Ok(self.recording.lock()?.deadlines.clone())
    }

    pub(super) fn written(&self) -> io::Result<Vec<u8>> {
        Ok(self.recording.lock()?.output.clone())
    }

    pub(super) fn set_write_limit(&mut self, limit: usize) -> io::Result<()> {
        self.recording.lock()?.write_limit = limit;
        Ok(())
    }

    pub(super) fn set_read_limit(&mut self, limit: usize) -> io::Result<()> {
        self.recording.lock()?.read_limit = limit;
        Ok(())
    }

    pub(super) fn interrupt_after_read(&mut self) -> io::Result<()> {
        self.recording.lock()?.reads_before_interrupt = Some(1);
        Ok(())
    }

    pub(super) fn disconnect(&mut self) -> io::Result<()> {
        self.recording.lock()?.disconnected = true;
        Ok(())
    }

    pub(super) fn lose_write_receipt(&mut self) -> io::Result<()> {
        self.recording.lock()?.write_failed_after_accept = true;
        Ok(())
    }

    pub(super) fn fail_capture(&mut self) -> io::Result<()> {
        self.recording.lock()?.capture_failed = true;
        Ok(())
    }

    pub(super) fn recorded(&self, direction: &str) -> io::Result<Vec<u8>> {
        let text =
            String::from_utf8(self.recording.lock()?.capture.clone()).map_err(io::Error::other)?;
        let marker = format!("\"direction\":\"{direction}\"");
        let mut result = Vec::new();
        for record in text.lines().filter(|record| record.contains(&marker)) {
            let hex = record
                .split("\"hex\":\"")
                .nth(1)
                .and_then(|tail| tail.strip_suffix("\"}"))
                .ok_or_else(|| io::Error::other("invalid recording entry"))?;
            for bytes in hex.as_bytes().as_chunks::<2>().0 {
                let digits = std::str::from_utf8(bytes).map_err(io::Error::other)?;
                result.push(u8::from_str_radix(digits, 16).map_err(io::Error::other)?);
            }
        }
        Ok(result)
    }
}

pub(super) fn packets(bytes: &[u8]) -> Vec<DumlFrame> {
    let mut packets = Vec::new();
    DumlParser::new().push(bytes, |frame| packets.push(frame));
    packets
}

pub(super) fn observed_poses() -> Vec<Vec<u8>> {
    packets(include_bytes!(
        "../../gimbal/fixtures/pitch-observations.bin"
    ))
    .into_iter()
    .map(|frame| frame.raw().to_vec())
    .collect()
}

pub(super) fn captured_hex_frames(text: &str) -> Vec<Vec<u8>> {
    text.lines()
        .map(|line| {
            let (pairs, tail) = line.as_bytes().as_chunks::<2>();
            assert!(tail.is_empty());
            pairs
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect()
        })
        .collect()
}

pub(super) fn seal_duml(bytes: &mut [u8]) {
    let length = bytes.len();
    let size = (length as u16).to_le_bytes();
    bytes[1] = size[0];
    bytes[2] = (bytes[2] & 0xfc) | size[1];
    bytes[3] = turret_dji::header_checksum(&bytes[..3]);
    let crc = turret_dji::frame_checksum(&bytes[..length - 2]).to_le_bytes();
    bytes[length - 2..].copy_from_slice(&crc);
}

pub(super) fn frame(pitch_rate: f32) -> ScopedControlFrame {
    ScopedControlFrame {
        session: SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new(GIMBAL_SCOPE),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        timing: None,
        admitted_timing: Some(pilotage_protocol::AdmittedTiming {
            admitted_at: MonoTimestamp::from_nanos(0),
            send_remaining: Duration::from_millis(100),
            active_remaining: Some(Duration::from_millis(100)),
            completion_timeout: None,
            window_bounded: true,
        }),
        profile_revision: 0,
        activation_revision: 0,
        payload: ControlPayload::default(),
        intent: Some(ControlIntent::GimbalRate(GimbalRateIntent {
            pitch_rate,
            yaw_rate: 0.0,
        })),
        actions: Vec::new(),
        action_ids: Vec::new(),
    }
}

//! One owner drains decoder output while it writes encoded input.

use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use pilotage_adapter_api::{
    CalibrationId, FrameStamper, GIMBAL_CAMERA, RawVideoFrame, UnstampedFrame,
};
use pilotage_adapter_dji::EncodedVideoChunk;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::{DjiVideoError, INPUT_GAP, SHUTDOWN_TIMEOUT, ppm::PpmDecoder};

const STDERR_LIMIT: usize = 4096;

pub(super) enum Next {
    Closed,
    Restart(Option<EncodedVideoChunk>),
}

struct Pending {
    bytes: Vec<u8>,
    offset: usize,
}

pub(super) struct Decoder {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    stderr: ChildStderr,
    diagnostics: Vec<u8>,
    stderr_open: bool,
    pending: Option<Pending>,
    continuity: Option<u64>,
    idle_at: Instant,
    ppm: PpmDecoder,
    dropped_frames: u64,
}

enum Event {
    Closed,
    Gap,
    Chunk(Option<EncodedVideoChunk>),
    Read(io::Result<usize>),
    Written(io::Result<usize>),
    Diagnostic(io::Result<usize>),
    Exited(io::Result<ExitStatus>),
}

impl Decoder {
    pub(super) fn start(path: &Path) -> Result<Self, DjiVideoError> {
        let mut child = Command::new(path)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "h264",
                "-probesize",
                "32",
                "-analyzeduration",
                "0",
                "-flags",
                "low_delay",
                "-threads",
                "1",
                "-i",
                "pipe:0",
                "-an",
                "-sn",
                "-dn",
                "-c:v",
                "ppm",
                "-pix_fmt",
                "rgb24",
                "-threads",
                "1",
                "-f",
                "image2pipe",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| DjiVideoError::Spawn {
                path: path.to_owned(),
                source,
            })?;
        Ok(Self {
            stdin: child
                .stdin
                .take()
                .ok_or(DjiVideoError::MissingPipe("stdin"))?,
            stdout: child
                .stdout
                .take()
                .ok_or(DjiVideoError::MissingPipe("stdout"))?,
            stderr: child
                .stderr
                .take()
                .ok_or(DjiVideoError::MissingPipe("stderr"))?,
            child,
            diagnostics: Vec::new(),
            stderr_open: true,
            pending: None,
            continuity: None,
            idle_at: Instant::now() + INPUT_GAP,
            ppm: PpmDecoder::default(),
            dropped_frames: 0,
        })
    }

    pub(super) fn accept(&mut self, chunk: EncodedVideoChunk) -> Result<(), DjiVideoError> {
        if chunk.bytes.len() > u16::MAX as usize {
            return Err(DjiVideoError::SegmentLimit(chunk.bytes.len()));
        }
        self.continuity = Some(chunk.continuity);
        self.idle_at = Instant::now() + INPUT_GAP;
        self.pending = (!chunk.bytes.is_empty()).then_some(Pending {
            bytes: chunk.bytes,
            offset: 0,
        });
        Ok(())
    }

    pub(super) async fn run(
        &mut self,
        chunks: &mut mpsc::Receiver<EncodedVideoChunk>,
        frames: &mpsc::Sender<RawVideoFrame>,
        stamper: &mut FrameStamper,
        origin: Instant,
    ) -> Result<Next, DjiVideoError> {
        let mut pixels = [0; 32 * 1024];
        let mut diagnostics = [0; 1024];
        loop {
            match self
                .event(chunks, frames, &mut pixels, &mut diagnostics)
                .await
            {
                Event::Closed | Event::Chunk(None) => return Ok(Next::Closed),
                Event::Gap => return Ok(Next::Restart(None)),
                Event::Chunk(Some(chunk)) => {
                    if self
                        .continuity
                        .is_some_and(|value| value != chunk.continuity)
                    {
                        return Ok(Next::Restart(Some(chunk)));
                    }
                    self.accept(chunk)?;
                }
                Event::Read(result) => {
                    let count = result.map_err(|source| io_error("read pixels", source))?;
                    if count == 0 {
                        self.ppm.finish()?;
                        return Err(self.exit_error("stdout closed".to_owned()));
                    }
                    self.emit(&pixels[..count], frames, stamper, origin)?;
                }
                Event::Written(result) => self.written(result)?,
                Event::Diagnostic(result) => {
                    let count = result.map_err(|source| io_error("read diagnostics", source))?;
                    self.stderr_open = count != 0;
                    self.retain_diagnostics(&diagnostics[..count]);
                }
                Event::Exited(result) => {
                    let status = result.map_err(|source| io_error("wait", source))?;
                    return Err(self.exit_error(status.to_string()));
                }
            }
        }
    }

    async fn event(
        &mut self,
        chunks: &mut mpsc::Receiver<EncodedVideoChunk>,
        frames: &mpsc::Sender<RawVideoFrame>,
        pixels: &mut [u8],
        diagnostics: &mut [u8],
    ) -> Event {
        let remaining = self
            .pending
            .as_ref()
            .map_or(&[][..], |p| &p.bytes[p.offset..]);
        tokio::select! {
            biased;
            () = frames.closed() => Event::Closed,
            () = tokio::time::sleep_until(self.idle_at) => Event::Gap,
            chunk = chunks.recv(), if self.pending.is_none() => Event::Chunk(chunk),
            result = self.stdout.read(pixels) => Event::Read(result),
            result = self.stderr.read(diagnostics), if self.stderr_open => Event::Diagnostic(result),
            result = self.stdin.write(remaining), if self.pending.is_some() => Event::Written(result),
            result = self.child.wait() => Event::Exited(result),
        }
    }

    fn written(&mut self, result: io::Result<usize>) -> Result<(), DjiVideoError> {
        let count = result.map_err(|source| io_error("write encoded bytes", source))?;
        if count == 0 {
            return Err(io_error(
                "write encoded bytes",
                io::ErrorKind::WriteZero.into(),
            ));
        }
        if let Some(pending) = &mut self.pending {
            pending.offset = pending.offset.wrapping_add(count);
            if pending.offset == pending.bytes.len() {
                self.pending = None;
            }
        }
        Ok(())
    }

    fn emit(
        &mut self,
        bytes: &[u8],
        frames: &mpsc::Sender<RawVideoFrame>,
        stamper: &mut FrameStamper,
        origin: Instant,
    ) -> Result<(), DjiVideoError> {
        for frame in self.ppm.push(bytes)? {
            // This is host observation time. The camera's sample time is unknown.
            let time_ns = origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            let frame = stamper.stamp(UnstampedFrame {
                width: frame.width,
                height: frame.height,
                pixel_format: "RGB_INT8".to_owned(),
                time_ns,
                rgb: frame.rgb,
                camera_id: u32::from(GIMBAL_CAMERA),
                calibration_id: Some(CalibrationId::NONE),
            });
            if let Err(mpsc::error::TrySendError::Full(_)) = frames.try_send(frame) {
                self.dropped_frames = self.dropped_frames.wrapping_add(1);
                tracing::debug!(
                    dropped_frames = self.dropped_frames,
                    "video consumer dropped a frame"
                );
            }
        }
        Ok(())
    }

    fn retain_diagnostics(&mut self, bytes: &[u8]) {
        self.diagnostics.extend_from_slice(bytes);
        let excess = self.diagnostics.len().saturating_sub(STDERR_LIMIT);
        self.diagnostics.drain(..excess);
    }

    fn exit_error(&self, status: String) -> DjiVideoError {
        DjiVideoError::Exited {
            status,
            stderr: String::from_utf8_lossy(&self.diagnostics).into_owned(),
        }
    }

    pub(super) async fn shutdown(&mut self) -> Result<(), DjiVideoError> {
        if self
            .child
            .try_wait()
            .map_err(|source| io_error("poll exit", source))?
            .is_some()
        {
            return Ok(());
        }
        self.child
            .start_kill()
            .map_err(|source| io_error("stop", source))?;
        tokio::time::timeout(SHUTDOWN_TIMEOUT, self.child.wait())
            .await
            .map_err(|_| DjiVideoError::ShutdownTimeout)?
            .map_err(|source| io_error("reap", source))?;
        Ok(())
    }
}

fn io_error(operation: &'static str, source: io::Error) -> DjiVideoError {
    DjiVideoError::Io { operation, source }
}

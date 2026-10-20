//! Decode the verified Mini 3 H.264 route on the host.

mod ppm;
mod process;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use pilotage_adapter_api::{
    CaptureClockMapping, FrameStamper, MeasurementClock, RawVideoFrame, SourceIncarnation,
};
use pilotage_adapter_dji::EncodedVideoChunk;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::Instrument;

use process::{Decoder, Next};

const FRAME_CAPACITY: usize = 4;
const INPUT_GAP: Duration = Duration::from_secs(1);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) struct DjiVideoConfig {
    pub(super) ffmpeg: PathBuf,
    pub(super) incarnation: SourceIncarnation,
    pub(super) origin: Instant,
    pub(super) keyframe_request: Arc<AtomicBool>,
}

#[derive(Debug, thiserror::Error)]
pub enum DjiVideoError {
    /// The configured decoder could not be started.
    #[error("cannot start DJI decoder {path:?}: {source}")]
    Spawn {
        /// The configured executable.
        path: PathBuf,
        /// The process creation error.
        #[source]
        source: std::io::Error,
    },
    /// Decoder pipe or process I/O failed.
    #[error("DJI decoder {operation} failed: {source}")]
    Io {
        /// The attempted operation.
        operation: &'static str,
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A requested process pipe was absent.
    #[error("DJI decoder did not provide its {0} pipe")]
    MissingPipe(&'static str),
    /// The decoder produced an invalid RGB frame.
    #[error("DJI decoder output is invalid: {0}")]
    Frame(#[from] ppm::PpmError),
    /// The decoder ended while the source remained open.
    #[error("DJI decoder exited with {status}: {stderr}")]
    Exited {
        /// The exit status or closed output pipe.
        status: String,
        /// The bounded tail of decoder diagnostics.
        stderr: String,
    },
    /// The encoded segment exceeded the transport envelope limit.
    #[error("DJI encoded segment has {0} bytes; the maximum is 65535")]
    SegmentLimit(usize),
    /// The child process did not exit before the shutdown deadline.
    #[error("DJI decoder did not stop within {SHUTDOWN_TIMEOUT:?}")]
    ShutdownTimeout,
}

pub(super) fn spawn(
    config: DjiVideoConfig,
    chunks: mpsc::Receiver<EncodedVideoChunk>,
) -> Result<(mpsc::Receiver<RawVideoFrame>, JoinHandle<()>), DjiVideoError> {
    let decoder = Decoder::start(&config.ffmpeg)?;
    let (frames_tx, frames_rx) = mpsc::channel(FRAME_CAPACITY);
    let task = tokio::spawn(
        async move {
            if let Err(error) = run(config, chunks, frames_tx, decoder).await {
                tracing::error!(%error, task = "dji-video-decoder", "video decoder stopped");
            }
            tracing::debug!(task = "dji-video-decoder", "task exited");
        }
        .instrument(tracing::info_span!("dji-video-decoder")),
    );
    Ok((frames_rx, task))
}

async fn run(
    config: DjiVideoConfig,
    mut chunks: mpsc::Receiver<EncodedVideoChunk>,
    frames: mpsc::Sender<RawVideoFrame>,
    mut decoder: Decoder,
) -> Result<(), DjiVideoError> {
    let mut stamper = FrameStamper::new(
        config.incarnation,
        MeasurementClock::HostMonotonic,
        CaptureClockMapping::Unavailable,
        BTreeMap::new(),
    );
    config.keyframe_request.store(true, Ordering::Release);
    loop {
        let result = decoder
            .run(&mut chunks, &frames, &mut stamper, config.origin)
            .await;
        let shutdown = decoder.shutdown().await;
        if let Err(error) = &shutdown {
            tracing::error!(%error, task = "dji-video-decoder", "decoder cleanup failed");
        }
        let next = result?;
        shutdown?;
        let Next::Restart(chunk) = next else {
            return Ok(());
        };
        stamper.reset_epoch();
        config.keyframe_request.store(true, Ordering::Release);
        let first = match chunk {
            Some(chunk) => chunk,
            None => tokio::select! {
                () = frames.closed() => return Ok(()),
                chunk = chunks.recv() => match chunk {
                    Some(chunk) => chunk,
                    None => return Ok(()),
                },
            },
        };
        decoder = Decoder::start(&config.ffmpeg)?;
        decoder.accept(first)?;
    }
}

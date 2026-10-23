//! The render worker thread and its latest-pose slot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use pilotage_adapter_api::{
    CaptureClockMapping, FrameStamper, GIMBAL_CAMERA, MeasurementClock, RawVideoFrame,
    SourceIncarnation, UnstampedFrame,
};
use tokio::sync::mpsc;

use super::fault;
use super::render::{CameraPose, Pinhole, Renderer};
use crate::config::{CameraConfig, TileSourceConfig};
use crate::geodesy::LocalFrame;
use crate::tiles::TileSource;
use crate::tiles::http::HttpTiles;
use crate::tiles::procedural::ProceduralTiles;

/// Frames waiting for the host media task.
const FRAME_QUEUE: usize = 2;
/// Every this many frames goes to the frame dump directory.
const DUMP_EVERY: u64 = 25;

#[derive(Default)]
struct Slot {
    pose: Option<CameraPose>,
    stop: bool,
}

#[derive(Default)]
struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
}

/// The handle of the render worker. Dropping it stops the worker after
/// its present frame.
pub struct CameraWorker {
    shared: Arc<Shared>,
}

/// What the worker needs besides the poses.
struct Job {
    config: CameraConfig,
    frame: LocalFrame,
    seed: u64,
    stamper: FrameStamper,
    frames: mpsc::Sender<RawVideoFrame>,
}

impl CameraWorker {
    /// Starts the worker thread and returns its handle and the frame
    /// receiver for the host media task.
    ///
    /// # Errors
    /// Returns the operating-system error when the thread cannot start.
    pub fn spawn(
        config: CameraConfig,
        frame: LocalFrame,
        seed: u64,
        incarnation: SourceIncarnation,
    ) -> std::io::Result<(Self, mpsc::Receiver<RawVideoFrame>)> {
        let (frames, receiver) = mpsc::channel(FRAME_QUEUE);
        let stamper = FrameStamper::new(
            incarnation,
            MeasurementClock::Simulation,
            CaptureClockMapping::identity(MeasurementClock::Simulation),
            BTreeMap::new(),
        );
        let shared = Arc::new(Shared::default());
        let job = Job {
            config,
            frame,
            seed,
            stamper,
            frames,
        };
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("sim-camera-render".to_owned())
            .spawn(move || run(&worker, job))?;
        Ok((Self { shared }, receiver))
    }

    /// Offers the latest camera pose; it replaces a pose not yet rendered.
    pub fn submit(&self, pose: CameraPose) {
        let mut slot = self
            .shared
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.pose = Some(pose);
        drop(slot);
        self.shared.wake.notify_one();
    }
}

impl Drop for CameraWorker {
    fn drop(&mut self) {
        let mut slot = self
            .shared
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.stop = true;
        drop(slot);
        self.shared.wake.notify_one();
    }
}

fn next_pose(shared: &Shared) -> Option<CameraPose> {
    let mut slot = shared.slot.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if slot.stop {
            return None;
        }
        if let Some(pose) = slot.pose.take() {
            return Some(pose);
        }
        slot = shared
            .wake
            .wait(slot)
            .unwrap_or_else(PoisonError::into_inner);
    }
}

fn tile_source(config: &TileSourceConfig, seed: u64) -> Box<dyn TileSource> {
    match config {
        TileSourceConfig::Procedural => Box::new(ProceduralTiles::new(seed)),
        TileSourceConfig::Http {
            template,
            cache_dir,
        } => match HttpTiles::new(template.clone(), cache_dir.clone()) {
            Ok(tiles) => Box::new(tiles),
            Err(error) => {
                tracing::error!(%error, "simulated camera uses the procedural texture");
                Box::new(ProceduralTiles::new(seed))
            }
        },
    }
}

fn run(shared: &Shared, mut job: Job) {
    // The HTTP client lives on this thread only: it must not be built or
    // dropped inside the host's async runtime.
    let mut tiles = tile_source(&job.config.tiles, job.seed);
    let camera = Pinhole::new(
        job.config.width,
        job.config.height,
        job.config.diagonal_fov_deg,
        job.config.max_zoom,
    );
    let mut rendered: u64 = 0;
    let mut dropped: u64 = 0;
    while let Some(pose) = next_pose(shared) {
        let mut rgb = Renderer::new(camera, job.frame, job.seed, tiles.as_mut()).render(&pose);
        let time_s = pose.time_ns as f64 / 1e9;
        for active in job
            .config
            .faults
            .iter()
            .filter(|fault| fault.active(time_s, pose.lat_deg, pose.lon_deg))
        {
            fault::apply(
                active.kind,
                &mut rgb,
                job.config.width as usize,
                job.config.height as usize,
            );
        }
        if let Some(dir) = job.config.frame_dump.as_deref()
            && rendered.is_multiple_of(DUMP_EVERY)
        {
            dump(dir, rendered, &job.config, &rgb);
        }
        rendered = rendered.wrapping_add(1);
        let frame = job.stamper.stamp(UnstampedFrame {
            width: job.config.width,
            height: job.config.height,
            pixel_format: "RGB_INT8".to_owned(),
            time_ns: pose.time_ns,
            rgb,
            camera_id: u32::from(GIMBAL_CAMERA),
            calibration_id: None,
        });
        match job.frames.try_send(frame) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                dropped = dropped.wrapping_add(1);
                tracing::debug!(
                    dropped,
                    "simulated camera frame dropped; the media task is behind"
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => break,
        }
    }
    tracing::debug!(rendered, dropped, "simulated camera worker stopped");
}

fn dump(dir: &Path, index: u64, config: &CameraConfig, rgb: &[u8]) {
    let path: PathBuf = dir.join(format!("frame-{index:06}.png"));
    let result = std::fs::create_dir_all(dir)
        .map_err(image::ImageError::IoError)
        .and_then(|()| {
            image::save_buffer(
                &path,
                rgb,
                config.width,
                config.height,
                image::ExtendedColorType::Rgb8,
            )
        });
    match result {
        Ok(()) => tracing::info!(path = %path.display(), "simulated camera frame saved"),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "simulated camera frame not saved")
        }
    }
}

#[cfg(test)]
mod tests;

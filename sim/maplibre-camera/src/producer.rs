//! The render thread: waits for each new pose, renders every camera from
//! it, and hands the frames to the link stamped with the pose's clock
//! reading.
//!
//! Rendering and readback block on the GPU, so they run on a thread of
//! their own rather than inside an async task; the thread enters the
//! runtime so the maps' tile requests run on it.

use std::time::{Duration, Instant};

use pilotage_sim_video::wire::{BridgeCameraCommand, BridgeFrame, BridgeGimbalPose};
use tracing::{debug, info, warn};

use crate::geometry::{BodyPose, Pointing};
use crate::link::LinkEnds;
use crate::render::Camera;
use crate::rig::Mount;

/// How often the producer logs its pose-to-frame latency.
const REPORT_INTERVAL: Duration = Duration::from_secs(5);
/// Where a gimbal points before anything states its pointing: ahead and
/// down, where a payload camera usually looks.
const DEFAULT_TILT_RAD: f64 = -0.5;

/// Pose-to-frame latency samples since the last report, milliseconds.
#[derive(Default)]
struct Latency {
    samples: Vec<f64>,
    render: Vec<f64>,
}

impl Latency {
    fn report(&mut self, dropped: u64) {
        let summary = |samples: &mut Vec<f64>| -> (f64, f64, f64) {
            samples.sort_by(f64::total_cmp);
            let at = |q: f64| {
                let index = ((samples.len() as f64 - 1.0) * q).round() as usize;
                samples.get(index).copied().unwrap_or(f64::NAN)
            };
            (
                at(0.5),
                at(0.95),
                samples.last().copied().unwrap_or(f64::NAN),
            )
        };
        let frames = self.samples.len();
        let (p50, p95, max) = summary(&mut self.samples);
        let (render_p50, render_p95, _) = summary(&mut self.render);
        info!(
            frames,
            dropped,
            pose_to_frame_p50_ms = p50,
            pose_to_frame_p95_ms = p95,
            pose_to_frame_max_ms = max,
            render_readback_p50_ms = render_p50,
            render_readback_p95_ms = render_p95,
            "sim camera latency"
        );
        self.samples.clear();
        self.render.clear();
    }
}

/// The gimbal's pointing and detent: the vehicle's reported gimbal when the
/// pose carries one, else the host's last camera command, else ahead and
/// down at the widest detent.
fn gimbal_pointing(
    reported: Option<BridgeGimbalPose>,
    command: Option<BridgeCameraCommand>,
) -> (Pointing, u32) {
    match (reported, command) {
        (Some(gimbal), _) => (
            Pointing {
                pan_rad: f64::from(gimbal.pan_rad),
                tilt_rad: f64::from(gimbal.tilt_rad),
            },
            gimbal.zoom_detent,
        ),
        (None, Some(command)) => (
            Pointing {
                pan_rad: f64::from(command.pan_rad),
                tilt_rad: f64::from(command.tilt_rad),
            },
            command.zoom_detent,
        ),
        (None, None) => (
            Pointing {
                pan_rad: 0.0,
                tilt_rad: DEFAULT_TILT_RAD,
            },
            0,
        ),
    }
}

/// Renders until the link ends. `handle` is the runtime the link and the
/// maps' tile requests run on.
pub fn run_blocking(
    mut cameras: Vec<Camera>,
    mut link: LinkEnds,
    fps: f64,
    handle: &tokio::runtime::Handle,
) {
    let _entered = handle.enter();
    let period = Duration::from_secs_f64(1.0 / fps);
    let start = Instant::now();
    let mut latency = Latency::default();
    let mut last_report = Instant::now();
    let mut refused = None;
    loop {
        if handle.block_on(link.poses.changed()).is_err() {
            return;
        }
        let tick = Instant::now();
        let Some(received) = *link.poses.borrow_and_update() else {
            continue;
        };
        let pose = match BodyPose::try_from(&received.pose) {
            Ok(pose) => pose,
            Err(error) => {
                if refused != Some(error) {
                    warn!(%error, "pose refused; no frames until a usable one");
                    refused = Some(error);
                }
                continue;
            }
        };
        refused = None;
        let command = *link.commands.borrow();
        for camera in &mut cameras {
            let (pointing, detent) = match camera.spec.mount {
                Mount::Body { .. } => (Pointing::default(), 0),
                Mount::Gimbal => gimbal_pointing(received.pose.gimbal, command),
            };
            let detent = camera.spec.detent(detent);
            let rendered = Instant::now();
            let rgb = match camera.render_blocking(&pose, pointing, detent, start.elapsed()) {
                Ok(rgb) => rgb.to_vec(),
                Err(error) => {
                    debug!(camera = camera.spec.camera_id, %error, "no frame");
                    continue;
                }
            };
            latency.render.push(rendered.elapsed().as_secs_f64() * 1e3);
            let frame = BridgeFrame {
                width: camera.spec.width,
                height: camera.spec.height,
                pixel_format: "RGB_INT8".to_owned(),
                sim_time_ns: received.pose.sim_time_ns,
                rgb,
                camera_id: camera.spec.camera_id,
                calibration_id: camera.spec.calibration_id(detent),
            };
            latency
                .samples
                .push(received.received_at.elapsed().as_secs_f64() * 1e3);
            if !link.offer(frame) {
                return;
            }
        }
        if last_report.elapsed() >= REPORT_INTERVAL {
            latency.report(link.dropped.load(std::sync::atomic::Ordering::Relaxed));
            last_report = Instant::now();
        }
        // Pacing, not polling: the next pose is rendered no sooner than one
        // period after this one.
        if let Some(rest) = period.checked_sub(tick.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

#[cfg(test)]
mod tests;

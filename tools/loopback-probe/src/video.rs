//! Decodes MJPEG video frames received on host-initiated uni streams
//! (ADR-0005 media = one uni stream per frame) and tracks the run's video
//! statistics: frame count, decoded dimensions, and arrival-to-display
//! latency.
//!
//! Decode uses the `image` crate's JPEG decoder; this binary owns I/O and
//! decode work (ADR-0002 sans-IO applies only to `crates/`).

use std::time::Duration;

use image::ImageFormat;
use tracing::warn;

use crate::metrics::Histogram;

/// One decoded video frame this probe has received, cheap enough to keep the
/// first/middle/last copies around for `--save-frames` without re-decoding.
#[derive(Clone)]
pub struct DecodedFrame {
    /// Decoded pixel width.
    pub width: u32,
    /// Decoded pixel height.
    pub height: u32,
    /// Decoded RGB8 pixel buffer, row-major, no padding.
    pub rgb: Vec<u8>,
}

/// Decodes one JPEG byte buffer into a [`DecodedFrame`].
///
/// Returns `None` (and logs) on a malformed JPEG, so one bad frame is skipped
/// rather than aborting the run.
#[must_use]
pub fn decode_jpeg(bytes: &[u8]) -> Option<DecodedFrame> {
    match image::load_from_memory_with_format(bytes, ImageFormat::Jpeg) {
        Ok(image) => {
            let rgb = image.to_rgb8();
            let (width, height) = rgb.dimensions();
            Some(DecodedFrame {
                width,
                height,
                rgb: rgb.into_raw(),
            })
        }
        Err(source) => {
            warn!(%source, "dropping undecodable video frame");
            None
        }
    }
}

/// Running video statistics accumulated over a probe run.
pub struct VideoStats {
    /// JPEG byte buffers received (whether or not they decoded).
    pub frames_received: u64,
    /// Frames received on the onboard FPV source (source id 0).
    pub fpv_received: u64,
    /// Frames received on the chase source (source id 1).
    pub chase_received: u64,
    /// Frames that decoded successfully.
    pub frames_decoded: u64,
    /// Frames that failed to decode.
    pub frames_decode_failed: u64,
    /// Arrival-to-decoded-display latency, measured from the client's own
    /// clock: the gap between this frame's uni stream finishing and the
    /// previous frame's, i.e. inter-arrival time, folded into a histogram so
    /// p50/p95/max frame cadence is reportable alongside true fps.
    pub inter_arrival: Histogram,
    /// Decoded width/height of the most recently decoded frame, if any.
    pub last_dims: Option<(u32, u32)>,
    /// The first successfully decoded frame, retained for `--save-frames`.
    pub first_frame: Option<DecodedFrame>,
    /// First successfully decoded frame from the onboard FPV source (id 0),
    /// retained so `--save-frames` can prove each source streamed separately.
    pub fpv_first_frame: Option<DecodedFrame>,
    /// First successfully decoded frame from the chase source (id 1), retained
    /// so `--save-frames` can prove each source streamed separately.
    pub chase_first_frame: Option<DecodedFrame>,
    /// The most recently decoded frame; at run end this is also the "last"
    /// frame for `--save-frames`.
    pub last_frame: Option<DecodedFrame>,
    /// A frame decoded roughly at the run's midpoint, for `--save-frames`.
    /// Updated by the caller (which knows the run's time budget), not by this
    /// module, since `VideoStats` has no notion of run duration.
    pub middle_frame: Option<DecodedFrame>,
}

/// Latency-histogram capacity for inter-arrival timing: comfortably above any
/// realistic frame count for a short demo run.
const VIDEO_HISTOGRAM_CAPACITY: usize = 10_000;

impl VideoStats {
    /// Constructs an empty video-stats accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            frames_received: 0,
            fpv_received: 0,
            chase_received: 0,
            frames_decoded: 0,
            frames_decode_failed: 0,
            inter_arrival: Histogram::new(VIDEO_HISTOGRAM_CAPACITY),
            last_dims: None,
            first_frame: None,
            fpv_first_frame: None,
            chase_first_frame: None,
            last_frame: None,
            middle_frame: None,
        }
    }

    /// Folds one arrived JPEG buffer into the stats: decodes it, records
    /// inter-arrival latency against `since_last`, counts the frame against its
    /// `source_id` (0 = FPV, 1 = chase), and updates the retained first/last
    /// frames.
    pub fn record(&mut self, source_id: u8, jpeg: &[u8], since_last: Option<Duration>) {
        self.record_at(source_id, jpeg, since_last, None);
    }

    /// Like [`Self::record`], additionally capturing this frame as the
    /// retained "middle" frame (for `--save-frames`) when `run_progress` is
    /// supplied and reports the run is past its halfway point for the first
    /// time. `run_progress` is `(elapsed, total_budget)`.
    pub fn record_at(
        &mut self,
        source_id: u8,
        jpeg: &[u8],
        since_last: Option<Duration>,
        run_progress: Option<(Duration, Duration)>,
    ) {
        self.frames_received = self.frames_received.saturating_add(1);
        match source_id {
            0 => self.fpv_received = self.fpv_received.saturating_add(1),
            1 => self.chase_received = self.chase_received.saturating_add(1),
            _ => {}
        }
        let Some(frame) = decode_jpeg(jpeg) else {
            self.frames_decode_failed = self.frames_decode_failed.saturating_add(1);
            return;
        };
        self.frames_decoded = self.frames_decoded.saturating_add(1);
        self.last_dims = Some((frame.width, frame.height));
        if let Some(gap) = since_last {
            self.inter_arrival.record(gap);
        }
        if self.first_frame.is_none() {
            self.first_frame = Some(frame.clone());
        }
        match source_id {
            0 if self.fpv_first_frame.is_none() => {
                self.fpv_first_frame = Some(frame.clone());
            }
            1 if self.chase_first_frame.is_none() => {
                self.chase_first_frame = Some(frame.clone());
            }
            _ => {}
        }
        if self.middle_frame.is_none()
            && let Some((elapsed, total)) = run_progress
            && elapsed >= total / 2
        {
            self.middle_frame = Some(frame.clone());
        }
        self.last_frame = Some(frame);
    }

    /// Average frames-per-second over `elapsed`, or `None` if no frames
    /// decoded or `elapsed` is zero.
    #[must_use]
    pub fn avg_fps(&self, elapsed: Duration) -> Option<f64> {
        if self.frames_decoded == 0 || elapsed.is_zero() {
            return None;
        }
        Some(self.frames_decoded as f64 / elapsed.as_secs_f64())
    }
}

impl Default for VideoStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Saves `frame` as a PNG file at `path`, creating parent directories as
/// needed.
///
/// # Errors
///
/// Returns a formatted error string on I/O or encode failure; this tool's
/// video-save path is best-effort proof output, not a correctness-critical
/// step, so callers log and continue rather than aborting the run.
pub async fn save_frame_png(frame: &DecodedFrame, path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| format!("creating {}: {source}", parent.display()))?;
    }
    let Some(buffer) = image::RgbImage::from_raw(frame.width, frame.height, frame.rgb.clone())
    else {
        return Err("decoded RGB buffer length does not match its own dimensions".to_string());
    };
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        buffer
            .save_with_format(&path, ImageFormat::Png)
            .map_err(|source| format!("saving {}: {source}", path.display()))
    })
    .await
    .map_err(|source| format!("save task panicked: {source}"))?
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{VideoStats, decode_jpeg};
    use std::time::Duration;

    /// Encodes a small synthetic RGB gradient to JPEG for round-trip tests.
    fn synthetic_jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                rgb.push((x % 256) as u8);
                rgb.push((y % 256) as u8);
                rgb.push(((x + y) % 256) as u8);
            }
        }
        let mut jpeg = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut jpeg);
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 80);
        encoder
            .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
            .expect("encodes");
        jpeg
    }

    #[test]
    fn decodes_valid_jpeg() {
        let jpeg = synthetic_jpeg(16, 12);
        let frame = decode_jpeg(&jpeg).expect("decodes");
        assert_eq!((frame.width, frame.height), (16, 12));
        assert_eq!(frame.rgb.len(), 16 * 12 * 3);
    }

    #[test]
    fn rejects_garbage_bytes() {
        assert!(decode_jpeg(&[0, 1, 2, 3]).is_none());
    }

    #[test]
    fn record_tracks_counts_and_dims() {
        let jpeg = synthetic_jpeg(8, 8);
        let mut stats = VideoStats::new();
        stats.record(0, &jpeg, None);
        stats.record(1, &jpeg, Some(Duration::from_millis(33)));
        assert_eq!(stats.frames_received, 2);
        assert_eq!(stats.fpv_received, 1, "one FPV frame counted");
        assert_eq!(stats.chase_received, 1, "one chase frame counted");
        assert_eq!(stats.frames_decoded, 2);
        assert_eq!(stats.frames_decode_failed, 0);
        assert_eq!(stats.last_dims, Some((8, 8)));
        assert_eq!(stats.inter_arrival.len(), 1);
    }

    #[test]
    fn record_counts_decode_failures_without_touching_dims() {
        let mut stats = VideoStats::new();
        stats.record(0, &[0xFF, 0x00], None);
        assert_eq!(stats.frames_received, 1);
        assert_eq!(stats.frames_decoded, 0);
        assert_eq!(stats.frames_decode_failed, 1);
        assert_eq!(stats.last_dims, None);
    }

    #[test]
    fn avg_fps_requires_decoded_frames_and_nonzero_elapsed() {
        let mut stats = VideoStats::new();
        assert_eq!(stats.avg_fps(Duration::from_secs(1)), None);
        stats.record(0, &synthetic_jpeg(4, 4), None);
        assert_eq!(stats.avg_fps(Duration::ZERO), None);
        let fps = stats.avg_fps(Duration::from_secs(2)).expect("has fps");
        assert!((fps - 0.5).abs() < 1e-9);
    }

    #[test]
    fn first_frame_is_retained_across_records() {
        let mut stats = VideoStats::new();
        stats.record(0, &synthetic_jpeg(4, 4), None);
        stats.record(0, &synthetic_jpeg(6, 6), None);
        let first = stats.first_frame.as_ref().expect("first frame retained");
        assert_eq!((first.width, first.height), (4, 4));
        let last = stats.last_frame.as_ref().expect("last frame retained");
        assert_eq!((last.width, last.height), (6, 6));
    }
}

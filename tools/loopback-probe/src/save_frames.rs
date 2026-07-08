//! `--save-frames DIR`: writes the first, a middle, and the last decoded
//! video frame as PNG files, plus one PNG per video source (`fpv.png`,
//! `chase.png`), giving visual proof the video downlink carried real,
//! decodable camera frames from each source.

use tracing::warn;

use crate::video::VideoStats;

/// Saves whichever of the first/middle/last decoded frames `video` retained,
/// as `first.png`, `middle.png`, and `last.png` under `dir`. Best-effort: a
/// missing frame (e.g. no video arrived at all) or a save failure only logs a
/// warning, since this is proof output, not a step the run's success depends
/// on.
pub async fn save_proof_frames(video: &VideoStats, dir: &str) {
    let dir = std::path::Path::new(dir);
    save_one(video.first_frame.as_ref(), &dir.join("first.png"), "first").await;
    save_one(
        video.middle_frame.as_ref(),
        &dir.join("middle.png"),
        "middle",
    )
    .await;
    save_one(video.last_frame.as_ref(), &dir.join("last.png"), "last").await;
    save_one(video.fpv_first_frame.as_ref(), &dir.join("fpv.png"), "fpv").await;
    save_one(
        video.chase_first_frame.as_ref(),
        &dir.join("chase.png"),
        "chase",
    )
    .await;
}

/// Saves one optional frame, logging a warning instead of failing the run if
/// the frame is absent or the save itself fails.
async fn save_one(frame: Option<&crate::video::DecodedFrame>, path: &std::path::Path, label: &str) {
    let Some(frame) = frame else {
        warn!(label, "no frame available to save (no video decoded yet)");
        return;
    };
    if let Err(message) = crate::video::save_frame_png(frame, path).await {
        warn!(label, %message, "failed to save proof frame");
    }
}

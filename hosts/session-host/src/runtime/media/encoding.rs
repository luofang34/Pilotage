//! JPEG encoding and the shared encoded-frame handoff.

use std::sync::Arc;

use jpeg_encoder::{ColorType, Encoder};
use pilotage_adapter_api::VideoCaptureStamp;
use pilotage_adapter_gazebo::RawVideoFrame;
use tracing::warn;

/// JPEG quality for the teleoperation preview.
const JPEG_QUALITY: u8 = 75;

/// One encoded frame shared across every connected client's handoff.
#[derive(Clone)]
pub(super) struct EncodedFrame {
    pub(super) jpeg: Arc<Vec<u8>>,
    pub(super) capture: VideoCaptureStamp,
    pub(super) received_at_ns: u64,
}

/// Encodes one raw RGB frame, skipping inputs the media contract cannot
/// represent instead of tearing down the shared pipeline.
pub(super) fn encode_jpeg(frame: &RawVideoFrame) -> Option<Vec<u8>> {
    if frame.pixel_format != "RGB_INT8" {
        warn!(
            format = frame.pixel_format,
            "skipping frame: only RGB_INT8 is supported by the media encoder"
        );
        return None;
    }
    let (width, height) = match (u16::try_from(frame.width), u16::try_from(frame.height)) {
        (Ok(w), Ok(h)) => (w, h),
        _ => {
            warn!(
                width = frame.width,
                height = frame.height,
                "skipping frame: dimensions exceed the JPEG encoder's 16-bit limit"
            );
            return None;
        }
    };
    let mut jpeg = Vec::new();
    let encoder = Encoder::new(&mut jpeg, JPEG_QUALITY);
    if let Err(error) = encoder.encode(&frame.rgb, width, height, ColorType::Rgb) {
        warn!(%error, "JPEG encode failed; skipping frame");
        return None;
    }
    Some(jpeg)
}

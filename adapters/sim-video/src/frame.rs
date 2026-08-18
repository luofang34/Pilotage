//! The stamped raw video frame every simulator producer converges on,
//! and the canonical video source identifiers.

use pilotage_adapter_api::VideoCaptureStamp;
use pilotage_timing::SimTick;

/// Identifier of the onboard FPV camera video source (source id 0).
pub const FPV_SOURCE_ID: &str = "onboard-fpv";
/// Identifier of the chase camera video source (source id 1).
pub const CHASE_SOURCE_ID: &str = "chase";
/// Identifier of the gimbal payload camera video source (source id 2).
pub const GIMBAL_SOURCE_ID: &str = "gimbal";
/// Wire source id of the onboard FPV camera.
pub const FPV_CAMERA: u8 = 0;
/// Wire source id of the chase camera.
pub const CHASE_CAMERA: u8 = 1;
/// Wire source id of the gimbal payload camera.
pub const GIMBAL_CAMERA: u8 = 2;

/// A decoded raw camera frame from a simulator video sidecar, carrying the
/// capture identity and clock mapping needed to trace it back to the
/// aircraft state (ADR-0020).
///
/// Exposed beside the `VehicleAdapter` trait rather than through it: frame
/// delivery is a streaming, backpressure-sensitive concern that does not fit
/// the pull-based `sample_telemetry` shape (ADR-0008). A frame is only ever
/// built by a [`crate::FrameStamper`], so its [`capture`](Self::capture) is
/// always fully formed.
#[derive(Debug, Clone)]
pub struct RawVideoFrame {
    /// Video source this frame came from: 0 = onboard FPV, 1 = chase, 2 =
    /// gimbal payload. Carried end to end so the host media pipeline and every
    /// reader can route each frame to the right video source (the wire
    /// `source_id` byte).
    pub source_id: u8,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Sidecar-reported pixel format (e.g. `"RGB_INT8"`).
    pub pixel_format: String,
    /// Simulation tick this frame was captured at (sidecar sim time, ns). Also
    /// carried in [`Self::capture`] as the capture stamp's acquisition time.
    pub tick: SimTick,
    /// Raw pixel bytes, row-major, no padding.
    pub rgb: Vec<u8>,
    /// Capture identity and clock mapping for this frame (ADR-0020).
    pub capture: VideoCaptureStamp,
}

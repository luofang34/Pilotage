//! The stamped raw video frame every producer converges on, the frame
//! stamper that is its sole constructor, and the canonical video source
//! identifiers (ADR-0020).
//!
//! A producer (a physics-engine sidecar, a window capture, a physical
//! camera gateway) reports each frame with a capture-time stamp but no
//! source identity or sequence; the [`FrameStamper`] supplies both. One
//! stamper is bound to a single adapter attachment: it holds that
//! attachment's opaque incarnation, advances a wrapping per-source
//! sequence, and attaches the caller-declared mapping from the capture
//! clock to the flight-state clock. Stamping is the sole constructor of
//! a [`RawVideoFrame`]'s [`VideoCaptureStamp`], so a frame can never
//! reach a reader without a fully-formed capture identity.

use std::collections::BTreeMap;

use super::{CalibrationId, CameraId, CaptureClockMapping, VideoCaptureStamp};
use crate::telemetry::{
    MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole,
};
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

/// One producer-reported frame before capture identity is assigned: the
/// pixels, their shape, the producer's capture-clock reading, and the
/// producer-local camera id. Producers convert their own wire or driver
/// type into this and hand it to a [`FrameStamper`].
#[derive(Debug, Clone)]
pub struct UnstampedFrame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Producer-reported pixel format (e.g. `"RGB_INT8"`).
    pub pixel_format: String,
    /// Capture time on the producer's clock, ns.
    pub time_ns: u64,
    /// Raw pixel bytes, row-major, no padding.
    pub rgb: Vec<u8>,
    /// Producer-local camera id (0 = FPV, 1 = chase, 2 = gimbal payload).
    pub camera_id: u32,
}

/// A raw camera frame carrying the capture identity and clock mapping
/// needed to trace it back to the aircraft state (ADR-0020).
///
/// Exposed beside the `VehicleAdapter` trait rather than through it: frame
/// delivery is a streaming, backpressure-sensitive concern that does not fit
/// the pull-based `sample_telemetry` shape (ADR-0008). A frame is only ever
/// built by a [`FrameStamper`], so its [`capture`](Self::capture) is always
/// fully formed.
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
    /// Producer-reported pixel format (e.g. `"RGB_INT8"`).
    pub pixel_format: String,
    /// Simulation tick this frame was captured at (producer capture time,
    /// ns). Also carried in [`Self::capture`] as the capture stamp's
    /// acquisition time.
    pub tick: SimTick,
    /// Raw pixel bytes, row-major, no padding.
    pub rgb: Vec<u8>,
    /// Capture identity and clock mapping for this frame (ADR-0020).
    pub capture: VideoCaptureStamp,
}

/// Assigns capture identity to every frame of one adapter attachment.
///
/// Bound to a single incarnation for its lifetime; a new attachment
/// constructs a new stamper with a fresh incarnation. The wrapping sequence
/// is tracked independently per routing source so the FPV and chase streams
/// order separately.
#[derive(Debug)]
pub struct FrameStamper {
    incarnation: SourceIncarnation,
    epoch: u32,
    clock: MeasurementClock,
    mapping: CaptureClockMapping,
    calibrations: BTreeMap<u32, CalibrationId>,
    next_sequence: BTreeMap<u8, u32>,
}

impl FrameStamper {
    /// Builds a stamper for one attachment identified by `incarnation`.
    /// `clock` declares the domain of the producer's capture stamps — a
    /// physics engine reports simulation time, a window-capture producer
    /// reports the host monotonic clock — and `mapping` maps that domain to
    /// the flight-state clock. `calibrations` binds a camera id to its
    /// published calibration id; a camera absent from the map stamps
    /// [`CalibrationId::NONE`], so a conformal consumer keeps its gate
    /// closed for it.
    #[must_use]
    pub fn new(
        incarnation: SourceIncarnation,
        clock: MeasurementClock,
        mapping: CaptureClockMapping,
        calibrations: BTreeMap<u32, CalibrationId>,
    ) -> Self {
        Self {
            incarnation,
            epoch: 0,
            clock,
            mapping,
            calibrations,
            next_sequence: BTreeMap::new(),
        }
    }

    /// Advances the attachment's epoch to mark a capture discontinuity (e.g. a
    /// producer reconnect), resetting every source's sequence so a receiver
    /// treats subsequent frames as a fresh, unordered start. Uses
    /// `wrapping_add(1)`, never `+= 1`, so a debug build cannot panic at the
    /// `u32` boundary.
    pub fn reset_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.next_sequence.clear();
    }

    /// Consumes one producer frame and returns it stamped with a complete
    /// capture identity: the attachment incarnation and epoch, a wrapping
    /// per-source sequence, the capture time, the camera identity, and the
    /// clock mapping.
    #[must_use]
    pub fn stamp(&mut self, frame: UnstampedFrame) -> RawVideoFrame {
        // The producer camera_id is a u32 for wire headroom, but only ids 0
        // (FPV) / 1 (chase) / 2 (gimbal payload) are assigned; an out-of-range
        // id saturates to u8::MAX so a reader routes it to no known source
        // rather than aliasing onto a valid one.
        let source_id = u8::try_from(frame.camera_id).unwrap_or(u8::MAX);
        let sequence = self.take_sequence(source_id);
        let stamp = MeasurementStamp {
            role: SourceRole::VideoCapture,
            // Sim camera frames arrive over an unauthenticated local link.
            integrity: SourceIntegrity::Unprotected,
            source_id: u64::from(source_id),
            source_incarnation: self.incarnation,
            source_epoch: self.epoch,
            sequence,
            acquired_at_ns: frame.time_ns,
            clock: self.clock,
        };
        RawVideoFrame {
            source_id,
            width: frame.width,
            height: frame.height,
            pixel_format: frame.pixel_format,
            tick: SimTick::new(frame.time_ns),
            rgb: frame.rgb,
            capture: VideoCaptureStamp {
                stamp,
                camera_id: CameraId(frame.camera_id),
                // The published calibration for this camera, if any; a camera
                // with no published calibration stamps NONE.
                calibration_id: self
                    .calibrations
                    .get(&frame.camera_id)
                    .copied()
                    .unwrap_or(CalibrationId::NONE),
                mapping: self.mapping,
            },
        }
    }

    /// Returns the next sequence for `source_id` and advances the stored value
    /// with `wrapping_add(1)`, so the counter cycles at the `u32` boundary
    /// instead of panicking in a debug build.
    fn take_sequence(&mut self, source_id: u8) -> u32 {
        let slot = self.next_sequence.entry(source_id).or_insert(0);
        let sequence = *slot;
        *slot = slot.wrapping_add(1);
        sequence
    }
}

#[cfg(test)]
mod tests;

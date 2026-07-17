//! Stamping raw sidecar camera frames with capture identity and a clock
//! mapping to the flight-state clock (ADR-0020).
//!
//! The sidecar reports each frame with a sim-time capture stamp but no
//! source identity or sequence; this module supplies both. One
//! [`FrameStamper`] is bound to a single adapter attachment: it holds that
//! attachment's opaque incarnation, advances a wrapping per-source sequence,
//! and attaches the caller-declared mapping from the sim capture clock to the
//! flight-state clock. Stamping is the sole constructor of a
//! [`RawVideoFrame`]'s [`VideoCaptureStamp`], so a frame can never reach a
//! reader without a fully-formed capture identity.

use std::collections::BTreeMap;

use pilotage_adapter_api::{
    CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, MeasurementStamp,
    SourceIncarnation, SourceIntegrity, SourceRole, VideoCaptureStamp,
};
use pilotage_timing::SimTick;

use crate::adapter::RawVideoFrame;
use crate::wire::BridgeFrame;

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
    mapping: CaptureClockMapping,
    calibrations: BTreeMap<u32, CalibrationId>,
    next_sequence: BTreeMap<u8, u32>,
}

impl FrameStamper {
    /// Builds a stamper for one attachment identified by `incarnation`, whose
    /// frames map to the flight-state clock via `mapping`. `calibrations` binds
    /// a camera id to its published calibration id; a camera absent from the
    /// map stamps [`CalibrationId::NONE`], so a conformal consumer keeps its
    /// gate closed for it.
    #[must_use]
    pub fn new(
        incarnation: SourceIncarnation,
        mapping: CaptureClockMapping,
        calibrations: BTreeMap<u32, CalibrationId>,
    ) -> Self {
        Self {
            incarnation,
            epoch: 0,
            mapping,
            calibrations,
            next_sequence: BTreeMap::new(),
        }
    }

    /// Advances the attachment's epoch to mark a capture discontinuity (e.g. a
    /// sidecar reconnect), resetting every source's sequence so a receiver
    /// treats subsequent frames as a fresh, unordered start. Uses
    /// `wrapping_add(1)`, never `+= 1`, so a debug build cannot panic at the
    /// `u32` boundary.
    pub fn reset_epoch(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.next_sequence.clear();
    }

    /// Consumes one sidecar frame and returns it stamped with a complete
    /// capture identity: the attachment incarnation and epoch, a wrapping
    /// per-source sequence, the sim capture time, the camera identity, and the
    /// clock mapping.
    #[must_use]
    pub fn stamp(&mut self, frame: BridgeFrame) -> RawVideoFrame {
        // The sidecar camera_id is a u32 for wire headroom, but only ids 0
        // (FPV) / 1 (chase) are assigned; an out-of-range id saturates to
        // u8::MAX so a reader routes it to no known source rather than
        // aliasing onto a valid one.
        let source_id = u8::try_from(frame.camera_id).unwrap_or(u8::MAX);
        let sequence = self.take_sequence(source_id);
        let stamp = MeasurementStamp {
            role: SourceRole::VideoCapture,
            // Sim camera frames arrive over unauthenticated gz transport.
            integrity: SourceIntegrity::Unprotected,
            source_id: u64::from(source_id),
            source_incarnation: self.incarnation,
            source_epoch: self.epoch,
            sequence,
            acquired_at_ns: frame.sim_time_ns,
            clock: MeasurementClock::Simulation,
        };
        RawVideoFrame {
            source_id,
            width: frame.width,
            height: frame.height,
            pixel_format: frame.pixel_format,
            tick: SimTick::new(frame.sim_time_ns),
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
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeMap;

    use super::FrameStamper;
    use pilotage_adapter_api::{
        CalibrationId, CameraId, CaptureClockMapping, MeasurementClock, SourceIncarnation,
    };

    use crate::wire::BridgeFrame;

    /// Test calibration binding: camera 0 resolves to calibration id 7.
    fn calibrations() -> BTreeMap<u32, CalibrationId> {
        let mut map = BTreeMap::new();
        map.insert(0, CalibrationId(7));
        map
    }

    fn bridge_frame(camera_id: u32, sim_time_ns: u64) -> BridgeFrame {
        BridgeFrame {
            width: 4,
            height: 4,
            pixel_format: "RGB_INT8".to_owned(),
            sim_time_ns,
            rgb: vec![0; 48],
            camera_id,
        }
    }

    fn stamper() -> FrameStamper {
        FrameStamper::new(
            SourceIncarnation::new([9; 16]),
            CaptureClockMapping::identity(MeasurementClock::Simulation),
            calibrations(),
        )
    }

    #[test]
    fn sequences_advance_per_source_from_zero() {
        let mut stamper = stamper();
        let fpv0 = stamper.stamp(bridge_frame(0, 100));
        let chase0 = stamper.stamp(bridge_frame(1, 110));
        let fpv1 = stamper.stamp(bridge_frame(0, 120));
        assert_eq!(fpv0.capture.stamp.sequence, 0);
        assert_eq!(fpv1.capture.stamp.sequence, 1, "FPV advances independently");
        assert_eq!(
            chase0.capture.stamp.sequence, 0,
            "chase starts its own count"
        );
        assert_eq!(fpv0.source_id, 0);
        assert_eq!(chase0.source_id, 1);
    }

    #[test]
    fn stamp_carries_capture_time_camera_and_mapping() {
        let mut stamper = stamper();
        let frame = stamper.stamp(bridge_frame(1, 4_242));
        assert_eq!(frame.capture.stamp.acquired_at_ns, 4_242);
        assert_eq!(frame.capture.stamp.clock, MeasurementClock::Simulation);
        assert_eq!(frame.capture.camera_id, CameraId(1));
        assert_eq!(frame.capture.calibration_id, CalibrationId::NONE);
        assert!(frame.capture.mapping.is_available());
        assert_eq!(frame.capture.mapping.error_bound_ns(), Some(0));
        assert_eq!(
            frame.capture.stamp.source_incarnation,
            SourceIncarnation::new([9; 16])
        );
    }

    #[test]
    fn reset_epoch_bumps_generation_and_restarts_sequences() {
        let mut stamper = stamper();
        let before = stamper.stamp(bridge_frame(0, 100));
        assert_eq!(before.capture.stamp.source_epoch, 0);
        assert_eq!(before.capture.stamp.sequence, 0);
        stamper.reset_epoch();
        let after = stamper.stamp(bridge_frame(0, 200));
        assert_eq!(after.capture.stamp.source_epoch, 1, "epoch advanced");
        assert_eq!(
            after.capture.stamp.sequence, 0,
            "sequence restarted after reset"
        );
    }

    #[test]
    fn sequence_wraps_at_the_u32_boundary() {
        let mut stamper = stamper();
        stamper.next_sequence.insert(0, u32::MAX);
        let wrap = stamper.stamp(bridge_frame(0, 1));
        let after = stamper.stamp(bridge_frame(0, 2));
        assert_eq!(wrap.capture.stamp.sequence, u32::MAX);
        assert_eq!(after.capture.stamp.sequence, 0, "wraps to 0, never panics");
    }

    #[test]
    fn unavailable_mapping_is_carried_verbatim() {
        let mut stamper = FrameStamper::new(
            SourceIncarnation::new([1; 16]),
            CaptureClockMapping::Unavailable,
            BTreeMap::new(),
        );
        let frame = stamper.stamp(bridge_frame(0, 1));
        assert!(!frame.capture.mapping.is_available());
        assert_eq!(frame.capture.mapping.error_bound_ns(), None);
    }

    #[test]
    fn calibration_is_stamped_for_bound_cameras_only() {
        let mut stamper = stamper();
        let fpv = stamper.stamp(bridge_frame(0, 1));
        let chase = stamper.stamp(bridge_frame(1, 2));
        assert_eq!(
            fpv.capture.calibration_id,
            CalibrationId(7),
            "the bound FPV camera stamps its published calibration"
        );
        assert_eq!(
            chase.capture.calibration_id,
            CalibrationId::NONE,
            "an unbound camera stamps NONE and stays gate-closed"
        );
    }
}

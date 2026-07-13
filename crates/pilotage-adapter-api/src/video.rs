//! Capture identity and clock mapping for streamed video frames (ADR-0020).
//!
//! A displayed camera frame is only useful for a conformal overlay if the
//! consumer can recover the aircraft state that corresponds to the *captured*
//! image, not to the moment the browser happened to receive it. That demands
//! two things travel with every frame: a traceable capture identity (which
//! source, which attachment, which frame) and an explicit statement of whether
//! the capture clock can be related to the flight-state clock at all, with a
//! quantified error when it can.
//!
//! The capture identity reuses the AV-01 [`MeasurementStamp`] vocabulary
//! wholesale rather than inventing a parallel one: a captured frame is just a
//! measurement group whose acquisition clock is the camera's. Only the
//! correlation to the flight-state clock is new, expressed by
//! [`CaptureClockMapping`].

use crate::telemetry::{MeasurementClock, MeasurementStamp};

/// The one canonical calibration identity, owned by the dependency-free `no_std`
/// `pilotage-calibration-id` leaf and re-exported here. A published camera
/// calibration and a synthetic-vision projection reference name the *same* type,
/// so there is no second `CalibrationId` and no lossy `u32` bridge between them.
/// `0` (its `NONE` sentinel, `!is_referenced()`) means no calibration identity
/// was published; a conformal consumer must treat that as "calibration
/// unavailable" and never assume a default. This crate depends only on the leaf,
/// not the whole geospatial contract, to name the id.
pub use pilotage_calibration_id::CalibrationId;

/// Compile-time proof that this crate's public `CalibrationId` *is* the leaf's,
/// not a second definition or a lossy `u32` mirror: a leaf value must be usable
/// as ours with no conversion. If anyone re-mints a local `CalibrationId`, this
/// identity coercion stops type-checking and the build fails here.
const _: fn(CalibrationId) -> pilotage_calibration_id::CalibrationId = |id| id;

/// Stable identity of the physical camera a frame came from, distinct from the
/// routing `source_id`: two attachments can reuse a routing slot over a
/// session's life, but a conformal overlay needs the camera whose intrinsics
/// and mounting produced this image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraId(pub u32);

/// Whether a frame's capture clock can be related to the flight-state clock,
/// and with what error.
///
/// This is the gate that decides whether a conformal overlay is even
/// admissible: absent a bounded mapping, the aircraft state that corresponds
/// to a captured image cannot be recovered, and a consumer must fall back to a
/// non-conformal presentation rather than draw the overlay against the wrong
/// state. The default is [`CaptureClockMapping::Unavailable`]: a mapping is
/// asserted only when an adapter can actually establish one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureClockMapping {
    /// No correlation between the capture clock and the flight-state clock is
    /// available. Conformal output must be gated off.
    #[default]
    Unavailable,
    /// A correlation with a bounded residual error is available.
    Bounded {
        /// Flight-state clock domain the capture time maps into.
        target: MeasurementClock,
        /// Nanoseconds added to the capture acquisition time to express it in
        /// `target`. Signed because the flight clock's origin may precede or
        /// follow the capture clock's.
        offset_ns: i64,
        /// Symmetric bound, in nanoseconds, on the residual error of the
        /// mapped time. This is the quantified clock error a consumer budgets
        /// against; it is meaningful only for this variant.
        error_bound_ns: u64,
    },
}

impl CaptureClockMapping {
    /// The identity mapping: the capture clock *is* the flight-state clock, so
    /// the capture time needs no offset and carries no residual error. Used
    /// when a single simulation clock stamps both the video frames and the
    /// telemetry (the Gazebo sidecar's sim time).
    #[must_use]
    pub const fn identity(clock: MeasurementClock) -> Self {
        Self::Bounded {
            target: clock,
            offset_ns: 0,
            error_bound_ns: 0,
        }
    }

    /// Whether a bounded mapping is present. A consumer gates conformal output
    /// on this being `true`.
    #[must_use]
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Bounded { .. })
    }

    /// The quantified error bound in nanoseconds when a mapping is available,
    /// or `None` when it is not.
    #[must_use]
    pub const fn error_bound_ns(self) -> Option<u64> {
        match self {
            Self::Bounded { error_bound_ns, .. } => Some(error_bound_ns),
            Self::Unavailable => None,
        }
    }

    /// The target flight-state clock a bounded mapping expresses times in, or
    /// `None` when no mapping is available.
    #[must_use]
    pub const fn target_clock(self) -> Option<MeasurementClock> {
        match self {
            Self::Bounded { target, .. } => Some(target),
            Self::Unavailable => None,
        }
    }

    /// Applies the mapping to a capture time, returning that instant expressed
    /// in the target clock.
    ///
    /// Returns `None` when the mapping is unavailable, or when the signed offset
    /// would carry the result outside the `u64` nanosecond range: a mapping
    /// that would overflow or underflow refuses rather than wrapping into a
    /// plausible-looking but wrong time.
    #[must_use]
    pub const fn map_capture_ns(self, capture_ns: u64) -> Option<u64> {
        match self {
            Self::Unavailable => None,
            Self::Bounded { offset_ns, .. } => {
                if offset_ns >= 0 {
                    capture_ns.checked_add(offset_ns.unsigned_abs())
                } else {
                    capture_ns.checked_sub(offset_ns.unsigned_abs())
                }
            }
        }
    }
}

/// Everything needed to trace one captured video frame back to the aircraft
/// state: the capture identity (an AV-01 [`MeasurementStamp`]), the camera and
/// calibration identities, and the mapping from the capture clock to the
/// flight-state clock.
///
/// The stamp's `source_id`, `source_epoch`, `sequence`, `acquired_at_ns` and
/// `clock` carry, respectively, the routing source, the attachment/reset
/// generation, the wrapping frame sequence, the capture time, and the capture
/// clock domain — the identity a receiver deduplicates and orders on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoCaptureStamp {
    /// Capture identity of this frame, in the AV-01 measurement vocabulary.
    pub stamp: MeasurementStamp,
    /// Physical camera this frame came from.
    pub camera_id: CameraId,
    /// Calibration that applies to the camera, or [`CalibrationId::NONE`].
    pub calibration_id: CalibrationId,
    /// Whether and how the capture clock relates to the flight-state clock.
    pub mapping: CaptureClockMapping,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{CalibrationId, CameraId, CaptureClockMapping, VideoCaptureStamp};
    use crate::telemetry::{MeasurementClock, MeasurementStamp, SourceIncarnation};

    fn stamp() -> MeasurementStamp {
        MeasurementStamp {
            source_id: 0,
            source_incarnation: SourceIncarnation::new([7; 16]),
            source_epoch: 0,
            sequence: 0,
            acquired_at_ns: 1_000,
            clock: MeasurementClock::Simulation,
        }
    }

    #[test]
    fn mapping_defaults_to_unavailable() {
        assert_eq!(
            CaptureClockMapping::default(),
            CaptureClockMapping::Unavailable
        );
        assert!(!CaptureClockMapping::default().is_available());
        assert_eq!(CaptureClockMapping::default().error_bound_ns(), None);
    }

    #[test]
    fn identity_mapping_is_available_with_zero_error() {
        let mapping = CaptureClockMapping::identity(MeasurementClock::Simulation);
        assert!(mapping.is_available());
        assert_eq!(mapping.error_bound_ns(), Some(0));
        assert_eq!(
            mapping,
            CaptureClockMapping::Bounded {
                target: MeasurementClock::Simulation,
                offset_ns: 0,
                error_bound_ns: 0,
            }
        );
    }

    #[test]
    fn bounded_mapping_reports_its_error_bound() {
        let mapping = CaptureClockMapping::Bounded {
            target: MeasurementClock::VehicleBoot,
            offset_ns: -42,
            error_bound_ns: 5_000,
        };
        assert!(mapping.is_available());
        assert_eq!(mapping.error_bound_ns(), Some(5_000));
        assert_eq!(mapping.target_clock(), Some(MeasurementClock::VehicleBoot));
    }

    #[test]
    fn map_capture_ns_applies_the_signed_offset() {
        let forward = CaptureClockMapping::Bounded {
            target: MeasurementClock::VehicleBoot,
            offset_ns: 100,
            error_bound_ns: 0,
        };
        assert_eq!(forward.map_capture_ns(1_000), Some(1_100));
        let backward = CaptureClockMapping::Bounded {
            target: MeasurementClock::VehicleBoot,
            offset_ns: -100,
            error_bound_ns: 0,
        };
        assert_eq!(backward.map_capture_ns(1_000), Some(900));
        assert_eq!(
            CaptureClockMapping::identity(MeasurementClock::Simulation).map_capture_ns(42),
            Some(42)
        );
    }

    #[test]
    fn map_capture_ns_refuses_to_wrap_at_the_u64_edges() {
        let overflow = CaptureClockMapping::Bounded {
            target: MeasurementClock::VehicleBoot,
            offset_ns: 1,
            error_bound_ns: 0,
        };
        assert_eq!(overflow.map_capture_ns(u64::MAX), None, "overflow refuses");
        let underflow = CaptureClockMapping::Bounded {
            target: MeasurementClock::VehicleBoot,
            offset_ns: -1,
            error_bound_ns: 0,
        };
        assert_eq!(underflow.map_capture_ns(0), None, "underflow refuses");
        assert_eq!(CaptureClockMapping::Unavailable.map_capture_ns(1_000), None);
    }

    #[test]
    fn calibration_none_is_not_referenced() {
        assert!(!CalibrationId::NONE.is_referenced());
        assert!(CalibrationId(1).is_referenced());
    }

    #[test]
    fn capture_stamp_holds_identity_and_mapping() {
        let capture = VideoCaptureStamp {
            stamp: stamp(),
            camera_id: CameraId(1),
            calibration_id: CalibrationId::NONE,
            mapping: CaptureClockMapping::Unavailable,
        };
        assert_eq!(capture.camera_id, CameraId(1));
        assert!(!capture.calibration_id.is_referenced());
        assert!(!capture.mapping.is_available());
    }
}

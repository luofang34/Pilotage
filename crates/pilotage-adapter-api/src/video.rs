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

/// Stable identity of the physical camera a frame came from, distinct from the
/// routing `source_id`: two attachments can reuse a routing slot over a
/// session's life, but a conformal overlay needs the camera whose intrinsics
/// and mounting produced this image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraId(pub u32);

/// Identity of the calibration (intrinsics/extrinsics) that applies to a
/// frame's camera. `0` means no calibration identity was published; a
/// conformal consumer must treat that as "calibration unavailable" and never
/// assume a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CalibrationId(pub u32);

impl CalibrationId {
    /// The sentinel meaning no calibration identity was published.
    pub const NONE: Self = Self(0);

    /// Whether a calibration identity was actually published for this frame.
    #[must_use]
    pub const fn is_published(self) -> bool {
        self.0 != 0
    }
}

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
    }

    #[test]
    fn calibration_none_is_unpublished() {
        assert!(!CalibrationId::NONE.is_published());
        assert!(CalibrationId(1).is_published());
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
        assert!(!capture.calibration_id.is_published());
        assert!(!capture.mapping.is_available());
    }
}

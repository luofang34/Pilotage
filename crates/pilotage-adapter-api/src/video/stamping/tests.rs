//! Frame-stamping behavior: per-source sequences, epoch resets, clock
//! declaration, and calibration binding.

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use super::{FrameStamper, UnstampedFrame};
use crate::telemetry::{MeasurementClock, SourceIncarnation};
use crate::video::{CalibrationId, CameraId, CaptureClockMapping};

/// Test calibration binding: camera 0 resolves to calibration id 7.
fn calibrations() -> BTreeMap<u32, CalibrationId> {
    let mut map = BTreeMap::new();
    map.insert(0, CalibrationId(7));
    map
}

fn frame(camera_id: u32, time_ns: u64) -> UnstampedFrame {
    UnstampedFrame {
        width: 4,
        height: 4,
        pixel_format: "RGB_INT8".to_owned(),
        time_ns,
        rgb: vec![0; 48],
        camera_id,
    }
}

fn stamper() -> FrameStamper {
    FrameStamper::new(
        SourceIncarnation::new([9; 16]),
        MeasurementClock::Simulation,
        CaptureClockMapping::identity(MeasurementClock::Simulation),
        calibrations(),
    )
}

#[test]
fn sequences_advance_per_source_from_zero() {
    let mut stamper = stamper();
    let fpv0 = stamper.stamp(frame(0, 100));
    let chase0 = stamper.stamp(frame(1, 110));
    let gimbal0 = stamper.stamp(frame(2, 115));
    let fpv1 = stamper.stamp(frame(0, 120));
    assert_eq!(fpv0.capture.stamp.sequence, 0);
    assert_eq!(fpv1.capture.stamp.sequence, 1, "FPV advances independently");
    assert_eq!(
        chase0.capture.stamp.sequence, 0,
        "chase starts its own count"
    );
    assert_eq!(gimbal0.capture.stamp.sequence, 0, "gimbal starts its own");
    assert_eq!(fpv0.source_id, 0);
    assert_eq!(chase0.source_id, 1);
    assert_eq!(
        gimbal0.source_id, 2,
        "camera_id 2 maps to the gimbal source"
    );
}

#[test]
fn stamp_carries_capture_time_camera_clock_and_mapping() {
    let mut stamper = stamper();
    let stamped = stamper.stamp(frame(1, 4_242));
    assert_eq!(stamped.capture.stamp.acquired_at_ns, 4_242);
    assert_eq!(stamped.capture.stamp.clock, MeasurementClock::Simulation);
    assert_eq!(stamped.capture.camera_id, CameraId(1));
    assert_eq!(stamped.capture.calibration_id, CalibrationId::NONE);
    assert!(stamped.capture.mapping.is_available());
    assert_eq!(stamped.capture.mapping.error_bound_ns(), Some(0));
    assert_eq!(
        stamped.capture.stamp.source_incarnation,
        SourceIncarnation::new([9; 16])
    );
}

#[test]
fn a_window_capture_declares_the_host_clock() {
    let mut stamper = FrameStamper::new(
        SourceIncarnation::new([2; 16]),
        MeasurementClock::HostMonotonic,
        CaptureClockMapping::Unavailable,
        BTreeMap::new(),
    );
    let stamped = stamper.stamp(frame(0, 77));
    assert_eq!(stamped.capture.stamp.clock, MeasurementClock::HostMonotonic);
}

#[test]
fn reset_epoch_bumps_generation_and_restarts_sequences() {
    let mut stamper = stamper();
    let before = stamper.stamp(frame(0, 100));
    assert_eq!(before.capture.stamp.source_epoch, 0);
    assert_eq!(before.capture.stamp.sequence, 0);
    stamper.reset_epoch();
    let after = stamper.stamp(frame(0, 200));
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
    let wrap = stamper.stamp(frame(0, 1));
    let after = stamper.stamp(frame(0, 2));
    assert_eq!(wrap.capture.stamp.sequence, u32::MAX);
    assert_eq!(after.capture.stamp.sequence, 0, "wraps to 0, never panics");
}

#[test]
fn unavailable_mapping_is_carried_verbatim() {
    let mut stamper = FrameStamper::new(
        SourceIncarnation::new([1; 16]),
        MeasurementClock::HostMonotonic,
        CaptureClockMapping::Unavailable,
        BTreeMap::new(),
    );
    let stamped = stamper.stamp(frame(0, 1));
    assert!(!stamped.capture.mapping.is_available());
    assert_eq!(stamped.capture.mapping.error_bound_ns(), None);
}

#[test]
fn calibration_is_stamped_for_bound_cameras_only() {
    let mut stamper = stamper();
    let fpv = stamper.stamp(frame(0, 1));
    let chase = stamper.stamp(frame(1, 2));
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

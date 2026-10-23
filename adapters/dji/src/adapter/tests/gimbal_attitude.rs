//! The gimbal attitude group: published once per angle push, only under a
//! declared yaw frame, with the identity the angle observations carry.

use pilotage_adapter_api::{MeasurementClock, SourceRole};
use pilotage_protocol::GimbalYawFrame;

use super::TestResult;
use super::harness::{Harness, observed_poses, packets};
use crate::{GimbalAngles, RouteAddress};

#[test]
fn a_declared_yaw_frame_publishes_the_gimbal_attitude_once_per_push() -> TestResult {
    let mut h = Harness::with_gimbal_frame(GimbalYawFrame::Earth);
    h.prime()?;
    let samples = h.telemetry(20).samples;
    let sample = samples.first().ok_or("no sample")?;
    let gimbal = sample.gimbal.ok_or("no gimbal attitude")?;
    let payload = sample.payload.as_ref().ok_or("missing payload")?;
    let pitch_stamp = payload
        .pitch
        .as_ref()
        .and_then(|pitch| pitch.stamp.as_ref())
        .ok_or("missing pitch stamp")?;
    let last = observed_poses().pop().ok_or("no poses")?;
    let frame = packets(&last).pop().ok_or("no frame")?;
    let angles = GimbalAngles::from_push(&frame, RouteAddress(2)).ok_or("not an angle push")?;
    let radians = |tenths: i16| f32::from(tenths).to_radians() / 10.0;
    let expected = super::super::airframe::quaternion_from_euler(
        radians(angles.roll_tenths),
        radians(angles.pitch_tenths),
        radians(angles.yaw_tenths),
    );
    assert_eq!(gimbal.quat_wxyz, expected);
    assert!(gimbal.rates_rps.iter().all(|rate| rate.is_nan()));
    assert_eq!(gimbal.reference.yaw_frame, GimbalYawFrame::Earth);
    assert!(!gimbal.reference.locks.yaw);
    assert_eq!(gimbal.stamp.role, SourceRole::PayloadDevice);
    assert_eq!(gimbal.stamp.clock, MeasurementClock::HostMonotonic);
    assert_eq!(gimbal.stamp.sequence, pitch_stamp.sequence);
    assert_eq!(gimbal.stamp.acquired_at_ns, pitch_stamp.acquired_at_ns);
    // No new push: the attitude is not repeated.
    let again = h.telemetry(25).samples;
    assert!(again.first().is_none_or(|sample| sample.gimbal.is_none()));
    Ok(())
}

#[test]
fn without_a_declared_frame_the_gimbal_angles_stay_off_the_attitude_group() -> TestResult {
    let mut h = Harness::new();
    h.prime()?;
    let samples = h.telemetry(20).samples;
    let sample = samples.first().ok_or("no sample")?;
    assert!(sample.payload.is_some());
    assert!(sample.gimbal.is_none());
    Ok(())
}

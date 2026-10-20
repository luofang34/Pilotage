use pilotage_adapter_api::{
    GimbalAttitudeSample, MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity,
    SourceRole,
};
use pilotage_protocol::wire;

use crate::{GimbalDeviceAttitude, ManagerControl};

/// Stable identity of one adapter's payload attachment.
#[derive(Debug)]
pub struct GimbalStamp {
    pub(super) incarnation: SourceIncarnation,
}

impl GimbalStamp {
    /// Start an attachment with a caller-generated source identity.
    #[must_use]
    pub fn new(incarnation: SourceIncarnation) -> Self {
        Self { incarnation }
    }

    fn stamp_for(&self, device: GimbalDeviceAttitude) -> MeasurementStamp {
        MeasurementStamp {
            role: SourceRole::PayloadDevice,
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: (u64::from(device.source_component) << 8) | u64::from(device.device_id),
            source_incarnation: self.incarnation,
            source_epoch: device.epoch as u32,
            sequence: device.sequence,
            acquired_at_ns: u64::from(device.time_boot_ms) * 1_000_000,
            clock: MeasurementClock::VehicleBoot,
        }
    }
}

/// Attitude and endpoint state from the same device report.
#[derive(Debug, Default)]
pub struct GimbalObservation {
    /// Device attitude, absent when no fresh report is available.
    pub attitude: Option<GimbalAttitudeSample>,
    /// Independent observations and retained command progress.
    pub payload: Option<wire::PayloadState>,
}

/// Read one device report without refreshing its receipt or source sequence.
#[must_use]
pub fn sample(control: &ManagerControl, stamp: &GimbalStamp) -> GimbalObservation {
    let device = control
        .device_cached()
        .filter(|report| report.received_at.elapsed() <= crate::sampling::WITHHOLD_AFTER);
    let attitude = device.and_then(|device| {
        Some(GimbalAttitudeSample {
            quat_wxyz: normalized_quaternion(device.quat_wxyz)?,
            rates_rps: device.rates_rps,
            reference: crate::gimbal_reference(device.flags),
            failure_flags: device.failure_flags,
            stamp: stamp.stamp_for(device),
        })
    });
    let payload = payload_state(control, stamp, device, attitude.as_ref());
    GimbalObservation { attitude, payload }
}

fn payload_state(
    control: &ManagerControl,
    stamp: &GimbalStamp,
    device: Option<GimbalDeviceAttitude>,
    attitude: Option<&GimbalAttitudeSample>,
) -> Option<wire::PayloadState> {
    let mut state = wire::PayloadState {
        endpoint_id: super::ENDPOINT.to_owned(),
        capability_revision: control.capabilities().revision,
        connection_incarnation: stamp.incarnation.into_bytes().to_vec(),
        connection_epoch: 1,
        command: control.snapshot_cached().map(|snapshot| {
            pilotage_adapter_api::payload_progress_to_wire(
                &snapshot.progress,
                Some(&snapshot.command),
                snapshot.updated_at,
            )
        }),
        ..Default::default()
    };
    if let Some(device) = device {
        let angles = attitude.map(|attitude| euler_of(attitude.quat_wxyz));
        let [roll_rate, pitch_rate, yaw_rate] = device.rates_rps;
        let stamp = wire_stamp(stamp.stamp_for(device));
        let received_at = device
            .received_at
            .checked_duration_since(control.started_at())
            .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
            .map(|nanos| wire::MonoTimestamp { nanos });
        let axis = |angle: Option<f32>, rate: f32| {
            let rate = rate.is_finite().then_some(rate);
            (angle.is_some() || rate.is_some()).then(|| wire::PayloadAxisObservation {
                angle_rad: angle,
                rate_rad_s: rate,
                stamp: Some(stamp.clone()),
                // Angular velocity and Euler angles need not share a reference.
                frame: wire::PayloadReferenceFrame::Unspecified as i32,
                source: wire::PayloadObservationSource::Transformed as i32,
                sampled_at: None,
                received_at,
                connection_epoch: 1,
            })
        };
        state.pitch = axis(angles.map(|[_, pitch, _]| pitch), pitch_rate);
        state.roll = axis(angles.map(|[roll, _, _]| roll), roll_rate);
        state.yaw = axis(angles.map(|[_, _, yaw]| yaw), yaw_rate);
    }
    (state.pitch.is_some()
        || state.roll.is_some()
        || state.yaw.is_some()
        || state.command.is_some())
    .then_some(state)
}

fn normalized_quaternion(value: [f32; 4]) -> Option<[f32; 4]> {
    let [w, x, y, z] = value.map(f64::from);
    let norm = (w * w + x * x + y * y + z * z).sqrt();
    // A device quaternion can contain roundoff; a scaled non-rotation cannot
    // acquire a level-attitude meaning through the Euler conversion.
    if !norm.is_finite() || (norm - 1.0).abs() > 0.1 {
        return None;
    }
    Some([w, x, y, z].map(|component| (component / norm) as f32))
}

fn wire_stamp(value: MeasurementStamp) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        source_id: value.source_id,
        source_incarnation: value.source_incarnation.into_bytes().to_vec(),
        source_epoch: value.source_epoch,
        sequence: value.sequence,
        acquired_at_ns: value.acquired_at_ns,
        role: wire::SourceRole::PayloadDevice as i32,
        integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
        clock: wire::MeasurementClock::VehicleBoot as i32,
    }
}

fn euler_of(q: [f32; 4]) -> [f32; 3] {
    let [w, x, y, z] = q;
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    [roll, pitch, yaw]
}

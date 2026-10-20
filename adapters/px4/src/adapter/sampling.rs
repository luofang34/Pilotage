//! Publish feedback for the selected payload with receive-side ordering intact.
use pilotage_adapter_api::{
    GimbalAttitudeSample, MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity,
    SourceRole,
};
use pilotage_mavlink::sampling::WITHHOLD_AFTER;

#[derive(Debug)]
pub(super) struct GimbalStamp {
    incarnation: SourceIncarnation,
}
impl GimbalStamp {
    pub(super) fn identity(&self) -> (SourceIncarnation, u32) {
        (self.incarnation, 1)
    }
    pub(super) fn new(incarnation: SourceIncarnation) -> Self {
        Self { incarnation }
    }
    fn stamp_for(&self, device: pilotage_mavlink::GimbalDeviceAttitude) -> MeasurementStamp {
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
impl super::Px4Adapter {
    pub(super) fn gimbal_attitude(&mut self) -> Option<GimbalAttitudeSample> {
        let device = self
            .gimbal
            .as_ref()?
            .device_cached()
            .filter(|report| report.received_at.elapsed() <= WITHHOLD_AFTER)?;
        Some(GimbalAttitudeSample {
            quat_wxyz: device.quat_wxyz,
            rates_rps: device.rates_rps,
            reference: pilotage_mavlink::gimbal_reference(device.flags),
            failure_flags: device.failure_flags,
            stamp: self.gimbal_stamp.stamp_for(device),
        })
    }
}

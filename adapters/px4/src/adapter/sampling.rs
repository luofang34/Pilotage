//! The gimbal payload-device lane: its stamp discipline and the sampled
//! orientation. The estimate and FC-state lanes are sampled by
//! [`pilotage_mavlink::sampling`].

use pilotage_adapter_api::{
    GimbalAttitudeSample, MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity,
    SourceRole,
};
use pilotage_mavlink::sampling::WITHHOLD_AFTER;

/// The payload-device stamp discipline for the gimbal lane. The device
/// reports `time_boot_ms` relative to its OWN boot, so a gimbal (or FC)
/// reboot regresses that value. Minting the stamp under a constant
/// identity and epoch — borrowing the FC arm incarnation with a fixed
/// epoch — would make the acquisition time and sequence run BACKWARDS
/// within one epoch, which a consumer cannot tell apart from a stale
/// replay. This carries a stable gimbal-source incarnation and opens a
/// NEW epoch whenever the boot time fails to advance (a reboot, or a u32
/// wrap), so within any one (incarnation, epoch) acquisition time is
/// monotonic and a reboot surfaces as a visible epoch change.
#[derive(Debug)]
pub(super) struct GimbalStamp {
    incarnation: SourceIncarnation,
    last_time_boot_ms: Option<u32>,
    epoch: u32,
}

impl GimbalStamp {
    pub(super) fn identity(&self) -> (SourceIncarnation, u32) {
        (self.incarnation, self.epoch)
    }

    /// A fresh discipline for one gimbal source instance. The incarnation
    /// is this adapter session's gimbal identity, distinct from the FC
    /// arm incarnation so a re-arm never reads as a gimbal reboot.
    pub(super) fn new(incarnation: SourceIncarnation) -> Self {
        Self {
            incarnation,
            last_time_boot_ms: None,
            epoch: 0,
        }
    }

    /// Mints the payload-device stamp for a device report as it is
    /// ingested into the provenance domain, advancing the epoch on a
    /// detected reboot — the first report, or a `time_boot_ms` that does
    /// not exceed the previous one. Re-observing the same boot time (the
    /// same report sampled twice) is not a reboot and keeps the epoch.
    pub(super) fn stamp_for(&mut self, time_boot_ms: u32) -> MeasurementStamp {
        let reboot = self
            .last_time_boot_ms
            .is_none_or(|prev| time_boot_ms < prev);
        if reboot {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.last_time_boot_ms = Some(time_boot_ms);
        MeasurementStamp {
            role: SourceRole::PayloadDevice,
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: 2,
            source_incarnation: self.incarnation,
            source_epoch: self.epoch,
            sequence: time_boot_ms,
            // The device reports its own boot-relative time; carry it as
            // the acquisition time under the vehicle-boot clock rather
            // than substituting a host stamp.
            acquired_at_ns: u64::from(time_boot_ms).saturating_mul(1_000_000),
            clock: MeasurementClock::VehicleBoot,
        }
    }
}

impl super::Px4Adapter {
    /// The latest gimbal-device orientation, withheld once stale so a
    /// frozen value never replays as live. Stamped under the
    /// payload-device role with the device's OWN boot clock
    /// (`time_boot_ms`), not host receive time: it is payload-device
    /// state, never a vehicle estimate and never eligible for control
    /// validation (LINK-04). The reset/epoch discipline lives in
    /// [`GimbalStamp`], so a device reboot opens a new epoch instead of
    /// regressing time under the old one.
    pub(super) fn gimbal_attitude(&mut self) -> Option<GimbalAttitudeSample> {
        let device = {
            let source = self.estimate.as_ref()?;
            let latest = source.state.lock().ok()?;
            latest
                .gimbal_device
                .filter(|report| report.received_at.elapsed() <= WITHHOLD_AFTER)?
        };
        Some(GimbalAttitudeSample {
            quat_wxyz: device.quat_wxyz,
            rates_rps: device.rates_rps,
            reference: pilotage_mavlink::gimbal_reference(device.flags),
            failure_flags: device.failure_flags,
            stamp: self.gimbal_stamp.stamp_for(device.time_boot_ms),
        })
    }
}

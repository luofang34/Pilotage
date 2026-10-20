use std::time::Duration;

use pilotage_adapter_api::{
    GimbalAttitudeSample, MeasurementClock, MeasurementStamp, SourceIncarnation, SourceIntegrity,
    SourceRole, TelemetryBatch, TelemetrySample,
};
use pilotage_protocol::{GimbalAxisLocks, GimbalReference, GimbalYawFrame, VehicleId};
use pilotage_timing::SimTick;

use crate::{DumlFrame, GimbalAngles, RouteAddress};

const MAX_AGE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy)]
pub(super) struct Pose {
    pub(super) angles: GimbalAngles,
    received: Duration,
    yaw_follow: bool,
}

pub(super) struct Observations {
    pub(super) latest: Option<Pose>,
    pub(super) consecutive: u8,
    incarnation: SourceIncarnation,
    sequence: u32,
    published_sequence: u32,
}

impl Observations {
    pub(super) fn new(incarnation: SourceIncarnation) -> Self {
        Self {
            latest: None,
            consecutive: 0,
            incarnation,
            sequence: 0,
            published_sequence: 0,
        }
    }

    pub(super) fn receive(&mut self, frame: &DumlFrame, now: Duration) {
        let angles = GimbalAngles::from_push(frame, RouteAddress(2))
            .or_else(|| GimbalAngles::from_push(frame, RouteAddress(10)));
        let Some(angles) = angles else {
            if frame.sender() == RouteAddress(4)
                && matches!(frame.receiver(), RouteAddress(2 | 10))
                && frame.flags() == 0
                && frame.command_set() == 4
                && frame.command_id() == 5
            {
                self.invalidate_continuity();
            }
            return;
        };
        if let Some(previous) = self.latest {
            let delta = angles.sequence.wrapping_sub(previous.angles.sequence);
            // Repeats and backwards packets cannot renew age or build admission.
            if delta == 0 || delta >= 0x8000 {
                return;
            }
        }
        let yaw_follow = frame.payload().get(6).is_some_and(|mode| mode >> 6 == 2);
        let continuous = self.age(now).is_some_and(|age| age <= MAX_AGE)
            && self.latest.is_some_and(Pose::qualifies);
        self.consecutive = if !yaw_follow || !(-50..=0).contains(&angles.pitch_tenths) {
            0
        } else if !continuous {
            1
        } else if self.latest.is_some_and(|pose| pose.received < now) {
            self.consecutive.saturating_add(1).min(3)
        } else {
            // Buffered reports in one read cycle are one host observation.
            self.consecutive
        };
        self.sequence = self.sequence.wrapping_add(1);
        self.latest = Some(Pose {
            angles,
            received: now,
            yaw_follow,
        });
    }

    pub(super) fn invalidate_continuity(&mut self) {
        self.consecutive = 0;
    }

    pub(super) fn age(&self, now: Duration) -> Option<Duration> {
        now.checked_sub(self.latest?.received)
    }

    pub(super) fn ready(&self, now: Duration) -> bool {
        self.consecutive >= 3
            && self.age(now).is_some_and(|age| age <= MAX_AGE)
            && self.latest.is_some_and(Pose::qualifies)
    }

    pub(super) fn sample(
        &mut self,
        vehicle: VehicleId,
        tick: SimTick,
        now: Duration,
        connection_valid: bool,
    ) -> TelemetryBatch {
        if !connection_valid
            || self.sequence == self.published_sequence
            || !self.age(now).is_some_and(|age| age <= MAX_AGE)
        {
            return TelemetryBatch::default();
        }
        let Some(pose) = self.latest.filter(|pose| pose.yaw_follow) else {
            return TelemetryBatch::default();
        };
        self.published_sequence = self.sequence;
        let sample = GimbalAttitudeSample {
            quat_wxyz: quaternion(pose.angles),
            rates_rps: [f32::NAN; 3],
            // DJI KeyGimbalAttitude defines NED attitude. The DUML decoder
            // DataGimbalGetPushParams uses offset 4 for yaw and offset 8 for
            // vehicle-relative yaw. Its byte-6 mode 2 is yaw-follow. Mini 3
            // captures match these fields; north has no independent calibration.
            reference: GimbalReference {
                yaw_frame: GimbalYawFrame::Earth,
                locks: GimbalAxisLocks {
                    roll: true,
                    pitch: true,
                    yaw: false,
                },
            },
            // No firmware fault bits have a verified mapping on this route.
            failure_flags: 0,
            stamp: MeasurementStamp {
                role: SourceRole::PayloadDevice,
                integrity: SourceIntegrity::ChecksummedOnly,
                source_id: 4,
                source_incarnation: self.incarnation,
                source_epoch: 0,
                sequence: self.sequence,
                acquired_at_ns: pose.received.as_nanos().min(u128::from(u64::MAX)) as u64,
                clock: MeasurementClock::HostMonotonic,
            },
        };
        TelemetryBatch {
            samples: vec![TelemetrySample {
                vehicle,
                tick,
                pose: None,
                speed: None,
                avionics: None,
                sim_truth: None,
                fc_state: None,
                gimbal: Some(sample),
                battery: None,
            }],
        }
    }
}

impl Pose {
    fn qualifies(self) -> bool {
        self.yaw_follow && (-50..=0).contains(&self.angles.pitch_tenths)
    }
}

fn quaternion(angles: GimbalAngles) -> [f32; 4] {
    let half_angle = std::f32::consts::PI / 3600.0;
    let (sr, cr) = (f32::from(angles.roll_tenths) * half_angle).sin_cos();
    let (sp, cp) = (f32::from(angles.pitch_tenths) * half_angle).sin_cos();
    let (sy, cy) = (f32::from(angles.yaw_tenths) * half_angle).sin_cos();
    [
        cr * cp * cy + sr * sp * sy,
        sr * cp * cy - cr * sp * sy,
        cr * sp * cy + sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
    ]
}

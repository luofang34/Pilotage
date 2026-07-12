//! Shared-memory source sampling and incarnation transitions.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, MeasurementClock,
    MeasurementStamp, Pose2d, SourceIncarnation, TelemetryBatch, TelemetrySample,
};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

use super::{WITHHOLD_AFTER, yaw_of};
use crate::error::AviateAdapterError;
use crate::shm::{GzStateSample, GzStateShm, ShmFreshness, ShmObservation};

const REATTACH_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub(super) struct ShmSource {
    shm: GzStateShm,
    freshness: ShmFreshness,
    instance: u8,
    epoch: u32,
    incarnation: SourceIncarnation,
    last_reattach_attempt: Option<Instant>,
}

impl ShmSource {
    pub(super) fn open(
        instance: u8,
        incarnation: SourceIncarnation,
    ) -> Result<Self, AviateAdapterError> {
        Ok(Self {
            shm: GzStateShm::open(instance)?,
            freshness: ShmFreshness::new(),
            instance,
            epoch: 1,
            incarnation,
            last_reattach_attempt: None,
        })
    }

    pub(super) fn current_pose(&mut self) -> Option<(f32, [f32; 3])> {
        self.usable_sample(Instant::now())
            .map(|sample| (yaw_of(sample.quat_wxyz) as f32, sample.pos_ned_m))
    }

    pub(super) fn tick(&self) -> u64 {
        self.shm
            .read()
            .map_or(0, |sample| sample.time_us.wrapping_mul(1_000))
    }

    pub(super) fn sample(&mut self, vehicle: VehicleId, arm_state: u32) -> TelemetryBatch {
        let now = Instant::now();
        let Some(sample) = self.usable_sample(now) else {
            return TelemetryBatch::default();
        };
        batch_from_sample(
            vehicle,
            arm_state,
            sample,
            self.instance,
            self.epoch,
            self.incarnation,
        )
    }

    fn usable_sample(&mut self, now: Instant) -> Option<GzStateSample> {
        let sample = self.shm.read();
        let usable = match sample {
            Some(sample) => match self.freshness.observe_at(sample.seq, sample.time_us, now) {
                ShmObservation::Advancing => true,
                ShmObservation::Unchanged(age) => age <= WITHHOLD_AFTER,
                ShmObservation::Quarantined => false,
            },
            None => false,
        };
        if usable {
            return sample;
        }
        let absent_stale =
            sample.is_none() && self.freshness.observe_absent_at(now) > WITHHOLD_AFTER;
        let frozen_or_quarantined = sample.is_some();
        if absent_stale || frozen_or_quarantined {
            self.try_reattach(now);
        }
        None
    }

    fn try_reattach(&mut self, now: Instant) {
        if self.last_reattach_attempt.is_some_and(|last| {
            now.checked_duration_since(last).unwrap_or_default() < REATTACH_INTERVAL
        }) {
            return;
        }
        self.last_reattach_attempt = Some(now);
        let Ok(candidate) = GzStateShm::open(self.instance) else {
            return;
        };
        if candidate.identity() == self.shm.identity() {
            return;
        }
        let Some(first) = candidate.read() else {
            return;
        };
        let mut freshness = ShmFreshness::new_at(now);
        if freshness.observe_at(first.seq, first.time_us, now) != ShmObservation::Advancing {
            return;
        }
        self.shm = candidate;
        self.freshness = freshness;
        self.epoch = self.epoch.wrapping_add(1);
        self.last_reattach_attempt = None;
        tracing::warn!(
            source_epoch = self.epoch,
            "Aviate SHM object entered a new attachment epoch"
        );
    }
}

fn batch_from_sample(
    vehicle: VehicleId,
    arm_state: u32,
    sample: GzStateSample,
    instance: u8,
    epoch: u32,
    incarnation: SourceIncarnation,
) -> TelemetryBatch {
    let heading = yaw_of(sample.quat_wxyz);
    let stamp = MeasurementStamp {
        source_id: u64::from(instance).wrapping_add(1),
        source_incarnation: incarnation,
        source_epoch: epoch,
        sequence: sample.seq,
        acquired_at_ns: sample.time_us.wrapping_mul(1_000),
        clock: MeasurementClock::Simulation,
    };
    let speed = f64::from(
        (sample.vel_ned_mps[0] * sample.vel_ned_mps[0]
            + sample.vel_ned_mps[1] * sample.vel_ned_mps[1])
            .sqrt(),
    );
    TelemetryBatch {
        samples: vec![TelemetrySample {
            vehicle,
            tick: SimTick::new(sample.time_us.wrapping_mul(1_000)),
            pose: Some(Pose2d {
                x: f64::from(sample.pos_ned_m[0]),
                y: f64::from(sample.pos_ned_m[1]),
                heading,
            }),
            speed: Some(speed),
            avionics: Some(AvionicsSample {
                attitude: Some(AvionicsAttitudeSample {
                    quat_wxyz: sample.quat_wxyz,
                    rates_rps: sample.rates_rps,
                    stamp,
                }),
                kinematics: Some(AvionicsKinematicsSample {
                    pos_ned_m: sample.pos_ned_m,
                    vel_ned_mps: sample.vel_ned_mps,
                    stamp,
                }),
                valid_flags: 0b1111,
                quality: 0,
                arm_state,
            }),
        }],
    }
}

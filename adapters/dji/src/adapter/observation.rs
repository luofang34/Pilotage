use std::time::Duration;

use pilotage_adapter_api::{SourceIncarnation, TelemetryBatch, TelemetrySample};
use pilotage_protocol::{VehicleId, wire};
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
    pub(super) fn incarnation(&self) -> SourceIncarnation {
        self.incarnation
    }

    pub(super) fn angle_feedback(
        &self,
        now: Duration,
        connection_valid: bool,
    ) -> turret_core::Capability<()> {
        use turret_core::{Capability, UnavailableReason};
        let Some(pose) = self.latest.filter(|pose| pose.yaw_follow) else {
            return Capability::Unknown;
        };
        if !connection_valid {
            Capability::Unavailable {
                capability: (),
                reason: UnavailableReason::Transport,
            }
        } else if now
            .checked_sub(pose.received)
            .is_some_and(|age| age <= MAX_AGE)
        {
            Capability::Available(())
        } else {
            Capability::Unavailable {
                capability: (),
                reason: UnavailableReason::MissingFeedback,
            }
        }
    }

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
        match self.payload_observation(pose) {
            Ok(payload) => TelemetryBatch {
                samples: vec![telemetry(vehicle, tick, payload)],
            },
            Err(error) => {
                tracing::error!(%error, "DJI angle observation rejected");
                TelemetryBatch::default()
            }
        }
    }

    pub(super) fn empty_state(&self) -> wire::PayloadState {
        wire::PayloadState {
            endpoint_id: "dji-camera".to_owned(),
            capability_revision: 1,
            connection_incarnation: self.incarnation.into_bytes().to_vec(),
            connection_epoch: 0,
            ..Default::default()
        }
    }

    fn payload_observation(&self, pose: Pose) -> Result<wire::PayloadState, turret_core::Error> {
        let received = super::execution::tick(pose.received);
        let stamp = wire::MeasurementStamp {
            source_id: 4,
            source_epoch: 0,
            sequence: self.sequence,
            acquired_at_ns: pose.received.as_nanos().min(u128::from(u64::MAX)) as u64,
            clock: wire::MeasurementClock::HostMonotonic as i32,
            source_incarnation: self.incarnation.into_bytes().to_vec(),
            role: wire::SourceRole::PayloadDevice as i32,
            integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
        };
        let axis = |tenths: i16| -> Result<_, turret_core::Error> {
            let observation = turret_core::Observation {
                value: turret_core::Angle::radians(f32::from(tenths).to_radians() / 10.0)?,
                source: turret_core::ObservationSource::DeviceMeasured,
                // Neither an absolute north reference nor device sample time
                // has been independently established for this captured route.
                frame: None,
                sampled_at: None,
                received_at: received,
                epoch: 0,
            };
            Ok(pilotage_adapter_api::payload_angle_observation_to_wire(
                &observation,
                stamp.clone(),
            ))
        };
        Ok(wire::PayloadState {
            pitch: Some(axis(pose.angles.pitch_tenths)?),
            roll: Some(axis(pose.angles.roll_tenths)?),
            yaw: Some(axis(pose.angles.yaw_tenths)?),
            ..self.empty_state()
        })
    }
}

impl Pose {
    fn qualifies(self) -> bool {
        self.yaw_follow && (-50..=0).contains(&self.angles.pitch_tenths)
    }
}

pub(super) fn telemetry(
    vehicle: VehicleId,
    tick: SimTick,
    payload: wire::PayloadState,
) -> TelemetrySample {
    TelemetrySample {
        vehicle,
        tick,
        payload: Some(payload),
        pose: None,
        speed: None,
        avionics: None,
        sim_truth: None,
        fc_state: None,
        gimbal: None,
        battery: None,
    }
}

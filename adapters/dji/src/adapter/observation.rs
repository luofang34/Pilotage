use std::time::Duration;

use pilotage_adapter_api::{SourceIncarnation, TelemetryBatch, TelemetrySample};
use pilotage_protocol::{VehicleId, wire};
use pilotage_timing::SimTick;
use turret_dji::Mini3ControlProfile;

use crate::{DumlFrame, GimbalAngles, RouteAddress};

use super::{DjiTelemetrySource, airframe::AirframeObservations, battery::BatteryObservations};

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
    profile: Mini3ControlProfile,
    incarnation: SourceIncarnation,
    pub(super) telemetry_source: DjiTelemetrySource,
    sequence: u32,
    published_sequence: u32,
    battery: BatteryObservations,
    airframe: AirframeObservations,
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
        let Some(pose) = self.latest.filter(|pose| pose.qualifies(self.profile)) else {
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

    pub(super) fn new(incarnation: SourceIncarnation, profile: Mini3ControlProfile) -> Self {
        Self {
            latest: None,
            consecutive: 0,
            profile,
            incarnation,
            telemetry_source: DjiTelemetrySource::new(incarnation),
            sequence: 0,
            published_sequence: 0,
            battery: BatteryObservations::default(),
            airframe: AirframeObservations::default(),
        }
    }

    /// Declares the verified north reference the airframe attitude is
    /// published under; `None` keeps it a display report.
    pub(super) fn set_heading_reference(
        &mut self,
        reference: Option<pilotage_protocol::HeadingReference>,
    ) {
        self.airframe.heading_reference = reference;
    }

    pub(super) fn receive(&mut self, frame: &DumlFrame, now: Duration) {
        let binding = self.model_deadline(now).map(|_| self.model_revision());
        self.battery.receive(frame, now, binding);
        let revision = self.model_revision();
        let was_identified = self.model_deadline(now).is_some();
        self.airframe.receive(frame, now);
        if self.model_revision() != revision {
            self.battery.invalidate_model();
        }
        if self.model_revision() != revision
            || (!was_identified && self.model_deadline(now).is_some())
        {
            self.invalidate_continuity();
        }
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
            && self.latest.is_some_and(|pose| pose.qualifies(self.profile));
        self.consecutive = if !yaw_follow
            || self
                .profile
                .validate_observed_pitch(angles.pitch_tenths)
                .is_err()
        {
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
        self.model_deadline(now).is_some()
            && self.consecutive >= 3
            && self.age(now).is_some_and(|age| age <= MAX_AGE)
            && self.latest.is_some_and(|pose| pose.qualifies(self.profile))
    }

    pub(super) fn model_deadline(&self, now: Duration) -> Option<Duration> {
        self.airframe.model_deadline(now)
    }

    pub(super) fn model_revision(&self) -> u64 {
        self.airframe.binding_revision()
    }

    pub(super) fn flight_cached(&self, now: Duration) -> Option<crate::flight::Mini3FlightReport> {
        self.airframe.flight_cached(now)
    }

    pub(super) fn flight_deadline(
        &self,
        command: crate::flight::BasicFlightCommand,
        now: Duration,
    ) -> Option<Duration> {
        let until = self.model_deadline(now)?;
        if command == crate::flight::BasicFlightCommand::Takeoff {
            let flight = self.airframe.flight_cached(now)?;
            if flight.motors_running || flight.is_flying {
                return None;
            }
            Some(until.min(self.battery.takeoff_deadline(now, self.model_revision())?))
        } else {
            Some(until)
        }
    }

    pub(super) fn motion_deadline(&self, now: Duration, enabled: bool) -> Option<Duration> {
        let flight = self.airframe.flight_cached(now)?;
        // The Mini 3 profile admits GPS attitude and the protocol's joystick
        // mode. Native takeoff, landing, RTH, and unknown modes retain priority.
        // https://github.com/o-gs/dji-firmware-tools/blob/195692263c2684cf1ddc4995f2736be6c0fb135e/comm_dissector/wireshark/dji-dumlv1-flyc.lua#L477-L515
        (flight.is_flying
            && flight.motors_running
            && (flight.mode == 6 || (enabled && flight.mode == 17)))
            .then(|| self.model_deadline(now))
            .flatten()
    }

    pub(super) fn expire_model(&mut self, now: Duration) {
        let revision = self.model_revision();
        self.airframe.expire_binding(now);
        if self.model_revision() != revision {
            self.invalidate_continuity();
            self.battery.invalidate_model();
        }
    }

    pub(super) fn sample(
        &mut self,
        vehicle: VehicleId,
        tick: SimTick,
        now: Duration,
        connection_valid: bool,
    ) -> TelemetryBatch {
        let payload = self.sample_payload(now, connection_valid);
        let battery = self
            .battery
            .sample(now, connection_valid, self.telemetry_source);
        let airframe = self
            .airframe
            .sample(now, connection_valid, self.telemetry_source);
        if payload.is_none() && battery.is_none() && airframe.is_none() {
            return TelemetryBatch::default();
        }
        let (avionics, fc_state, speed) = airframe
            .map(|sample| (Some(sample.avionics), Some(sample.fc_state), sample.speed))
            .unwrap_or_default();
        TelemetryBatch {
            samples: vec![TelemetrySample {
                battery,
                avionics,
                fc_state,
                speed,
                ..telemetry(vehicle, tick, payload)
            }],
        }
    }

    fn sample_payload(
        &mut self,
        now: Duration,
        connection_valid: bool,
    ) -> Option<wire::PayloadState> {
        if !connection_valid
            || self.sequence == self.published_sequence
            || !self.age(now).is_some_and(|age| age <= MAX_AGE)
        {
            return None;
        }
        let pose = self.latest.filter(|pose| pose.qualifies(self.profile))?;
        self.published_sequence = self.sequence;
        match self.payload_observation(pose) {
            Ok(payload) => Some(payload),
            Err(error) => {
                tracing::error!(%error, "DJI angle observation rejected");
                None
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
    fn qualifies(self, profile: Mini3ControlProfile) -> bool {
        self.yaw_follow
            && profile
                .validate_observed_pitch(self.angles.pitch_tenths)
                .is_ok()
    }
}

pub(super) fn telemetry(
    vehicle: VehicleId,
    tick: SimTick,
    payload: Option<wire::PayloadState>,
) -> TelemetrySample {
    TelemetrySample {
        flight_procedure: None,
        vehicle,
        tick,
        payload,
        pose: None,
        speed: None,
        avionics: None,
        sim_truth: None,
        fc_state: None,
        gimbal: None,
        battery: None,
    }
}

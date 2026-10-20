use std::time::Duration;

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsSample, FcStateSample, MeasurementClock, MeasurementStamp,
    MotorStateSample, SourceIncarnation, SourceIntegrity, SourceRole,
};
use pilotage_protocol::MotorState;

use crate::DumlFrame;
use crate::flight::{Mini3FlightReport, osd_payload};

const MAX_AGE: Duration = Duration::from_millis(300);
const SOURCE_ID: u64 = 0x0e_03_43;
const ESTIMATOR_QUALITY_UNUSABLE: u32 = 2;

#[derive(Clone, Copy)]
pub(super) struct AirframeReport {
    angles_tenths: [i16; 3],
    motor_state: MotorState,
    flight: Mini3FlightReport,
}

impl AirframeReport {
    pub(super) fn from_push(frame: &DumlFrame) -> Option<Self> {
        let payload = osd_payload(frame)?;
        let flight = Mini3FlightReport::from_push(frame)?;
        // This Mini 3 profile infers the legacy OSD angle prefix. It does not
        // establish axis directions or a north reference.
        // https://github.com/o-gs/dji-firmware-tools/blob/195692263c2684cf1ddc4995f2736be6c0fb135e/comm_dissector/wireshark/dji-dumlv1-flyc.lua#L886-L912
        let angles_tenths = [
            i16::from_le_bytes([payload[24], payload[25]]),
            i16::from_le_bytes([payload[26], payload[27]]),
            i16::from_le_bytes([payload[28], payload[29]]),
        ];
        if angles_tenths
            .iter()
            .any(|angle| !(-1_800..=1_800).contains(angle))
        {
            return None;
        }
        Some(Self {
            angles_tenths,
            flight,
            motor_state: if flight.motors_running {
                MotorState::Running
            } else {
                MotorState::Stopped
            },
        })
    }

    fn avionics(self, stamp: MeasurementStamp) -> AvionicsSample {
        let [pitch_rad, roll_rad, yaw_rad] = self
            .angles_tenths
            .map(|angle| (f32::from(angle) / 10.0).to_radians());
        AvionicsSample {
            attitude: Some(AvionicsAttitudeSample::ReportedEuler {
                pitch_rad,
                roll_rad,
                yaw_rad,
                stamp,
            }),
            kinematics: None,
            baro: None,
            geodetic: None,
            magnetic_variation: None,
            estimator_status_stamp: None,
            valid_flags: 0,
            // No estimator status accompanies this fallback value.
            quality: ESTIMATOR_QUALITY_UNUSABLE,
        }
    }
}

pub(super) struct AirframeSample {
    pub(super) avionics: AvionicsSample,
    pub(super) fc_state: FcStateSample,
}

#[derive(Default)]
pub(super) struct AirframeObservations {
    latest: Option<(AirframeReport, Duration)>,
    last_sequence: Option<u16>,
    binding_revision: u64,
    sequence: u32,
    published_sequence: u32,
}

impl AirframeObservations {
    pub(super) fn receive(&mut self, frame: &DumlFrame, now: Duration) {
        if osd_payload(frame).is_none() {
            return;
        }
        // A replacement FC can restart its sequence. Negative identity evidence
        // closes the binding even when it cannot advance the stream watermark.
        if Mini3FlightReport::reports_other_model(frame) {
            self.invalidate_binding();
        }
        if let Some(previous) = self.last_sequence {
            let delta = frame.sequence().wrapping_sub(previous);
            if delta == 0 || delta >= 0x8000 {
                return;
            }
        }
        self.last_sequence = Some(frame.sequence());
        let Some(report) = AirframeReport::from_push(frame) else {
            self.invalidate_binding();
            return;
        };
        self.sequence = self.sequence.wrapping_add(1);
        self.latest = Some((report, now));
    }

    pub(super) fn binding_revision(&self) -> u64 {
        self.binding_revision
    }

    pub(super) fn model_deadline(&self, now: Duration) -> Option<Duration> {
        let (_, received) = self.latest?;
        let until = received.checked_add(MAX_AGE)?;
        (now >= received && now < until).then_some(until)
    }

    pub(super) fn flight_cached(&self, now: Duration) -> Option<Mini3FlightReport> {
        self.model_deadline(now)?;
        Some(self.latest?.0.flight)
    }

    pub(super) fn expire_binding(&mut self, now: Duration) {
        if self.model_deadline(now).is_none() {
            self.invalidate_binding();
        }
    }

    fn invalidate_binding(&mut self) {
        if self.latest.take().is_some() {
            self.binding_revision = self.binding_revision.wrapping_add(1);
        }
    }

    pub(super) fn sample(
        &mut self,
        now: Duration,
        connection_valid: bool,
        incarnation: SourceIncarnation,
    ) -> Option<AirframeSample> {
        let (report, received) = self.latest?;
        if !connection_valid
            || self.sequence == self.published_sequence
            || !now.checked_sub(received).is_some_and(|age| age <= MAX_AGE)
        {
            return None;
        }
        self.published_sequence = self.sequence;
        let stamp = MeasurementStamp {
            role: SourceRole::FcState,
            integrity: SourceIntegrity::ChecksummedOnly,
            // Route 0e is a relay. This key identifies its OSD stream.
            source_id: SOURCE_ID,
            source_incarnation: incarnation,
            source_epoch: 0,
            sequence: self.sequence,
            acquired_at_ns: received.as_nanos().min(u128::from(u64::MAX)) as u64,
            clock: MeasurementClock::HostMonotonic,
        };
        Some(AirframeSample {
            avionics: report.avionics(MeasurementStamp {
                // The role identifies the report, not estimator authorization.
                role: SourceRole::OperationalEstimate,
                ..stamp
            }),
            fc_state: FcStateSample {
                arm_state: 0,
                last_command: None,
                landed_state: None,
                motor_state: Some(MotorStateSample {
                    state: report.motor_state,
                    stamp,
                }),
                stamp: None,
            },
        })
    }
}

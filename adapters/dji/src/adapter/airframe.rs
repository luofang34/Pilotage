use std::time::Duration;

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsSample, FcStateSample, LandingApproachSample, MeasurementClock,
    MeasurementStamp, MotorStateSample, SourceIntegrity, SourceRole,
};
use pilotage_protocol::MotorState;

use super::DjiTelemetrySource;
use crate::DumlFrame;
use crate::flight::{Mini3Fix, Mini3FlightReport, Mini3PositionReport, osd_payload};

mod navigation;

use navigation::{Navigation, NavigationGroups};

const MAX_AGE: Duration = Duration::from_millis(300);
const SOURCE_ID: u64 = 0x0e_03_43;
const ESTIMATOR_QUALITY_UNUSABLE: u32 = 2;
/// The estimator validity bit for an authorized attitude.
const VALID_ATTITUDE: u32 = 0b0001;

#[derive(Clone, Copy)]
pub(super) struct AirframeReport {
    angles_tenths: [i16; 3],
    motor_state: MotorState,
    flight: Mini3FlightReport,
    approach: Option<(f32, f32)>,
    /// The FC's own fix, when the report carries a satellite solution.
    position: Option<Mini3Fix>,
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
            approach: downward_range(payload, angles_tenths),
            motor_state: if flight.motors_running {
                MotorState::Running
            } else {
                MotorState::Stopped
            },
            position: Mini3PositionReport::from_push(frame).and_then(|report| report.fix()),
        })
    }

    /// The attitude is a device report with an unverified reference and
    /// stays unauthorized, unless the operator declared the reference they
    /// verified: then it is published as an authorized attitude under it.
    /// A fix adds the FC's own position and velocity under the
    /// authorization its satellite use supports.
    fn avionics(
        self,
        stamp: MeasurementStamp,
        navigation: Option<NavigationGroups>,
        verified_heading: bool,
    ) -> AvionicsSample {
        let [pitch_rad, roll_rad, yaw_rad] = self
            .angles_tenths
            .map(|angle| (f32::from(angle) / 10.0).to_radians());
        let attitude = Some(if verified_heading {
            AvionicsAttitudeSample::Canonical {
                quat_wxyz: quaternion_from_euler(roll_rad, pitch_rad, yaw_rad),
                rates_rps: None,
                stamp,
            }
        } else {
            AvionicsAttitudeSample::ReportedEuler {
                pitch_rad,
                roll_rad,
                yaw_rad,
                stamp,
            }
        });
        let attitude_flag = if verified_heading { VALID_ATTITUDE } else { 0 };
        match navigation {
            Some(groups) => AvionicsSample {
                attitude,
                kinematics: Some(groups.kinematics),
                baro: None,
                geodetic: Some(groups.geodetic),
                magnetic_variation: None,
                estimator_status_stamp: Some(stamp),
                valid_flags: groups.valid_flags | attitude_flag,
                quality: groups.quality,
            },
            None => AvionicsSample {
                attitude,
                kinematics: None,
                baro: None,
                geodetic: None,
                magnetic_variation: None,
                estimator_status_stamp: None,
                valid_flags: attitude_flag,
                // No estimator status accompanies this fallback value.
                quality: ESTIMATOR_QUALITY_UNUSABLE,
            },
        }
    }
}

pub(super) struct AirframeSample {
    pub(super) avionics: AvionicsSample,
    pub(super) fc_state: FcStateSample,
    /// Horizontal ground speed from the FC's fix, metres per second.
    pub(super) speed: Option<f64>,
}

#[derive(Default)]
pub(super) struct AirframeObservations {
    latest: Option<(AirframeReport, Duration)>,
    last_sequence: Option<u16>,
    binding_revision: u64,
    sequence: u32,
    published_sequence: u32,
    navigation: Navigation,
    /// The north reference the operator verified; `None` publishes the
    /// attitude as a display report.
    pub(super) heading_reference: Option<pilotage_protocol::HeadingReference>,
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
        self.navigation.observe_motors(report.flight.motors_running);
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
        self.navigation.reset();
    }

    pub(super) fn sample(
        &mut self,
        now: Duration,
        connection_valid: bool,
        source: DjiTelemetrySource,
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
            source_incarnation: source.incarnation,
            source_epoch: source.epoch,
            sequence: self.sequence,
            acquired_at_ns: received.as_nanos().min(u128::from(u64::MAX)) as u64,
            clock: MeasurementClock::HostMonotonic,
        };
        let estimate = MeasurementStamp {
            // The role identifies the report, not estimator authorization.
            role: SourceRole::OperationalEstimate,
            ..stamp
        };
        let navigation = report
            .position
            .as_ref()
            .and_then(|fix| self.navigation.groups(fix, estimate, self.binding_revision));
        let speed = navigation.as_ref().map(|groups| groups.speed_mps);
        Some(AirframeSample {
            avionics: report.avionics(estimate, navigation, self.heading_reference.is_some()),
            speed,
            fc_state: FcStateSample {
                arm_state: 0,
                last_command: None,
                landed_state: None,
                landing_approach: report.approach.map(
                    |(surface_distance_m, vertical_speed_down_mps)| LandingApproachSample {
                        surface_distance_m,
                        vertical_speed_down_mps,
                        is_flying: report.flight.is_flying,
                        stamp,
                    },
                ),
                motor_state: Some(MotorStateSample {
                    state: report.motor_state,
                    stamp,
                }),
                stamp: None,
            },
        })
    }
}

fn downward_range(payload: &[u8; 70], angles: [i16; 3]) -> Option<(f32, f32)> {
    // The Mini 3 OSD prefix carries a decimetre downward range. Its use/error
    // flags must accompany the value; a cached range alone is not proximity.
    // Keep slant measurements out of the near-ground landing gesture.
    let flags = u32::from_le_bytes([payload[32], payload[33], payload[34], payload[35]]);
    if flags & 0x10 == 0
        || flags & 0x20000 != 0
        || !(1..255).contains(&payload[41])
        || angles[0].abs() > 150
        || angles[1].abs() > 150
    {
        return None;
    }
    let speed = i16::from_le_bytes([payload[22], payload[23]]);
    (speed.unsigned_abs() <= 200)
        .then_some((f32::from(payload[41]) / 10.0, f32::from(speed) / 10.0))
}

/// The body-FRD to world-NED quaternion (w, x, y, z) of roll, pitch and yaw
/// applied in the yaw-pitch-roll order the reported angles describe.
pub(super) fn quaternion_from_euler(roll_rad: f32, pitch_rad: f32, yaw_rad: f32) -> [f32; 4] {
    let (sr, cr) = (roll_rad / 2.0).sin_cos();
    let (sp, cp) = (pitch_rad / 2.0).sin_cos();
    let (sy, cy) = (yaw_rad / 2.0).sin_cos();
    [
        cr * cp * cy + sr * sp * sy,
        sr * cp * cy - cr * sp * sy,
        cr * sp * cy + sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
    ]
}

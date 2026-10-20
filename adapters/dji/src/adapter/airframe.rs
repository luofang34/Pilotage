use std::time::Duration;

use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsSample, MeasurementClock, MeasurementStamp, SourceIncarnation,
    SourceIntegrity, SourceRole,
};

use crate::{DumlFrame, RouteAddress};

const MAX_AGE: Duration = Duration::from_millis(300);
const SOURCE_ID: u64 = 0x0e_03_43;
const ESTIMATOR_QUALITY_UNUSABLE: u32 = 2;

#[derive(Clone, Copy)]
pub(super) struct AirframeReport {
    sequence: u16,
    angles_tenths: [i16; 3],
}

impl AirframeReport {
    pub(super) fn from_push(frame: &DumlFrame) -> Option<Self> {
        if frame.version() != 1
            || frame.sender() != RouteAddress(14)
            || frame.receiver() != RouteAddress(2)
            || frame.flags() != 0
            || frame.command_set() != 3
            || frame.command_id() != 0x43
        {
            return None;
        }
        let payload: &[u8; 70] = frame.payload().try_into().ok()?;
        // This Mini 3 profile infers the legacy OSD angle prefix. It does not
        // establish axis directions, a north reference, or the remaining fields.
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
            sequence: frame.sequence(),
            angles_tenths,
        })
    }
}

#[derive(Default)]
pub(super) struct AirframeObservations {
    latest: Option<(AirframeReport, Duration)>,
    sequence: u32,
    published_sequence: u32,
}

impl AirframeObservations {
    pub(super) fn receive(&mut self, frame: &DumlFrame, now: Duration) {
        let Some(report) = AirframeReport::from_push(frame) else {
            return;
        };
        if let Some((previous, _)) = self.latest {
            let delta = report.sequence.wrapping_sub(previous.sequence);
            if delta == 0 || delta >= 0x8000 {
                return;
            }
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.latest = Some((report, now));
    }

    pub(super) fn sample(
        &mut self,
        now: Duration,
        connection_valid: bool,
        incarnation: SourceIncarnation,
    ) -> Option<AvionicsSample> {
        let (report, received) = self.latest?;
        if !connection_valid
            || self.sequence == self.published_sequence
            || !now.checked_sub(received).is_some_and(|age| age <= MAX_AGE)
        {
            return None;
        }
        self.published_sequence = self.sequence;
        let [pitch_rad, roll_rad, yaw_rad] = report
            .angles_tenths
            .map(|angle| (f32::from(angle) / 10.0).to_radians());
        Some(AvionicsSample {
            attitude: Some(AvionicsAttitudeSample::ReportedEuler {
                pitch_rad,
                roll_rad,
                yaw_rad,
                stamp: MeasurementStamp {
                    // The role identifies the report, not estimator authorization.
                    role: SourceRole::OperationalEstimate,
                    integrity: SourceIntegrity::ChecksummedOnly,
                    // Route 0e is a relay. This key identifies its OSD stream.
                    source_id: SOURCE_ID,
                    source_incarnation: incarnation,
                    source_epoch: 0,
                    sequence: self.sequence,
                    acquired_at_ns: received.as_nanos().min(u128::from(u64::MAX)) as u64,
                    clock: MeasurementClock::HostMonotonic,
                },
            }),
            kinematics: None,
            baro: None,
            geodetic: None,
            magnetic_variation: None,
            estimator_status_stamp: None,
            valid_flags: 0,
            // No estimator status accompanies this fallback value.
            quality: ESTIMATOR_QUALITY_UNUSABLE,
        })
    }
}

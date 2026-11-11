//! The vehicle status the mission engine waits on before it arms and
//! launches, read from the wire telemetry the principal receives: the
//! flight controller's readiness to arm and, under its estimator's
//! authorization, its position validity, its height above the local
//! origin, and its airspeed.

use navigate_contract::MonotonicNanos;
use pilotage_mission::VehicleStatus;
use pilotage_protocol::wire;

/// The authorization bit of a valid position.
const VALID_POSITION: u32 = 0b00_0100;
/// The authorization bit of a valid height above the local origin, which a
/// barometer keeps valid without a position fix.
const VALID_HEIGHT: u32 = 0b01_0000;
/// The authorization bit of a valid airspeed.
const VALID_AIRSPEED: u32 = 0b10_0000;

impl super::MissionTask {
    /// Hands the mission engine the vehicle status a telemetry sample
    /// states. A sample that states no readiness to arm hands nothing, so
    /// a flight controller that never states it leaves the engine as it
    /// was.
    pub(super) fn report_vehicle_status(
        &mut self,
        sample: &wire::TelemetrySample,
        now: MonotonicNanos,
    ) {
        let Some(status) = vehicle_status(sample, now) else {
            return;
        };
        if let Some(mission) = self.mission.as_mut() {
            mission.on_vehicle_status(status);
        }
    }
}

/// The vehicle status of one telemetry sample received at `now`, if the
/// sample states the readiness to arm. The validity bits authorize only
/// under an estimator status stamp, and each value only with the stamp of
/// the group that carries it.
fn vehicle_status(sample: &wire::TelemetrySample, now: MonotonicNanos) -> Option<VehicleStatus> {
    let ready_to_arm = sample.fc_state.as_ref()?.ready_to_arm?;
    let mut status = VehicleStatus::ready(ready_to_arm, now);
    let Some(avionics) = sample
        .avionics
        .as_ref()
        .filter(|avionics| avionics.estimator_status_stamp.is_some())
    else {
        return Some(status);
    };
    let valid = |bit: u32| avionics.valid_flags & bit != 0;
    status.position_valid = Some(valid(VALID_POSITION));
    status.height_above_origin_m = (valid(VALID_HEIGHT) && avionics.kinematics_stamp.is_some())
        .then(|| -f64::from(avionics.pos_d_m))
        .filter(|height| height.is_finite());
    status.airspeed_mps = (valid(VALID_AIRSPEED) && avionics.airspeed_stamp.is_some())
        .then(|| f64::from(avionics.indicated_airspeed_mps))
        .filter(|airspeed| airspeed.is_finite() && *airspeed >= 0.0);
    Some(status)
}

#[cfg(test)]
mod tests;

//! Whether the vehicle left flight on its own: the flight controller's
//! stamped arm and landed reports, read for a disarm after an arm or a
//! landing after the vehicle was in the air. A plan never lands, so either
//! one during the flown phases ends the run with its reason.

use navigate_contract::MonotonicNanos;
use pilotage_mission::VehicleStatus;
use pilotage_protocol::wire;

/// The estimate authorization bit of a valid position (bit 2 of the
/// avionics group's `valid_flags`).
const VALID_POSITION: u32 = 0b0100;

/// What the flight controller has reported during this run.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct FlightWatch {
    armed: bool,
    in_air: bool,
    /// The flight controller has sent a stamped state report in this run.
    reports: bool,
    /// The arm that waits for the flight controller's answer.
    pub(super) arm: super::arm_receipt::ArmReceipt,
}

impl FlightWatch {
    /// Takes one report; the reason the run must end when it shows the
    /// vehicle leaving the flight it was in.
    pub(super) fn observe(&mut self, fc: &wire::FcState) -> Option<&'static str> {
        self.reports |= fc.stamp.is_some();
        if fc.stamp.is_some() {
            match fc.arm_state {
                2 => self.armed = true,
                1 if self.armed => return Some("the vehicle disarmed outside the plan"),
                _ => {}
            }
        }
        let landed = fc
            .landed_state
            .as_ref()
            .filter(|landed| landed.stamp.is_some())
            .and_then(|landed| wire::LandedState::try_from(landed.state).ok());
        match landed {
            Some(wire::LandedState::InAir) => self.in_air = true,
            Some(wire::LandedState::Landing | wire::LandedState::OnGround) if self.in_air => {
                return Some("the vehicle landed outside the plan");
            }
            _ => {}
        }
        None
    }
}

impl FlightWatch {
    /// Whether the flight controller reports its state, so that its answer
    /// to an arm can arrive. A vehicle with no flight-controller state
    /// report (the reference skiff) has no answer but the command path.
    pub(super) const fn reports_state(&self) -> bool {
        self.reports
    }
}

impl super::MissionTask {
    /// Hands the mission engine the vehicle status a telemetry sample
    /// states: the flight controller's readiness to arm, whether its
    /// estimate states a valid position and, under that authorization, its
    /// height above its local origin. A sample that states no readiness
    /// hands nothing, so a flight controller that never states it leaves
    /// the engine as before.
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
/// sample states the readiness to arm. The flags authorize only under an
/// estimator status stamp; the group has no height bit of its own, so the
/// local height is read under the position authorization.
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
    let position_valid = avionics.valid_flags & VALID_POSITION != 0;
    status.position_valid = Some(position_valid);
    status.height_above_origin_m = (position_valid && avionics.kinematics_stamp.is_some())
        .then(|| -f64::from(avionics.pos_d_m))
        .filter(|height| height.is_finite());
    Some(status)
}

#[cfg(test)]
mod tests;

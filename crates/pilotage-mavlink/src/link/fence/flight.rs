//! Flight-procedure preconditions checked at the final transport write.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pilotage_adapter_api::SourceIncarnation;

use crate::sampling::operational_pose;
use crate::{LandedState, LinkState};

/// Selected source and observation limits for one queued takeoff command.
#[derive(Debug, Clone, Copy)]
pub struct TakeoffWriteRequirement {
    system_id: u8,
    component_id: u8,
    source_id: u64,
    source_incarnation: SourceIncarnation,
    source_epoch: u32,
    required_custom_mode: u32,
    maximum_age: Duration,
    home_origin_offset_m: Option<f32>,
}

impl TakeoffWriteRequirement {
    /// Retains the source identity and flight-mode precondition at admission.
    #[must_use]
    pub fn new(state: &LinkState, required_custom_mode: u32, maximum_age: Duration) -> Self {
        Self {
            system_id: state.system_id,
            component_id: state.component_id,
            source_id: state.source_id,
            source_incarnation: state.source_incarnation,
            source_epoch: state.source_epoch,
            required_custom_mode,
            maximum_age,
            home_origin_offset_m: state.fc_reports.relative_altitude.and_then(|altitude| {
                state
                    .kinematics
                    .map(|position| altitude.altitude_m + position.pos_ned_m[2])
                    .filter(|offset| offset.is_finite())
            }),
        }
    }

    fn permits(&self, state: &LinkState, now: Instant) -> bool {
        let fresh = |received| {
            now.checked_duration_since(received)
                .is_some_and(|age| age < self.maximum_age)
        };
        self.system_id == state.system_id
            && self.component_id == state.component_id
            && self.source_id == state.source_id
            && self.source_incarnation == state.source_incarnation
            && self.source_epoch == state.source_epoch
            && state.pending_reset.is_none()
            && state.last_heartbeat.is_some_and(fresh)
            && state.heartbeat_armed == Some(true)
            && state.fc_reports.custom_mode == Some(self.required_custom_mode)
            && state
                .landed_state_cached(now, self.maximum_age)
                .is_some_and(|report| report.state == LandedState::OnGround)
            && state
                .attitude
                .is_some_and(|report| fresh(report.received_at))
            && state
                .kinematics
                .is_some_and(|report| fresh(report.received_at))
            && self.pose_and_datum_current(state, now)
    }

    fn pose_and_datum_current(&self, state: &LinkState, now: Instant) -> bool {
        let Some(altitude) = state.relative_altitude_cached(now, self.maximum_age) else {
            return false;
        };
        let Some(pose) = operational_pose(state) else {
            return false;
        };
        let current_offset = altitude.altitude_m + pose.pos_ned_m[2];
        // The native target uses the home datum captured at admission. A new
        // home reference must not change that target while it waits to write.
        pose.yaw_rad.is_finite()
            && pose.pos_ned_m.iter().all(|value| value.is_finite())
            && self
                .home_origin_offset_m
                .is_some_and(|expected| (current_offset - expected).abs() <= 0.5)
    }
}

#[derive(Debug, Clone)]
pub(super) struct FlightGuard {
    pub(super) state: Arc<Mutex<LinkState>>,
    pub(super) requirement: TakeoffWriteRequirement,
}

impl FlightGuard {
    pub(super) fn write(
        &self,
        operation: impl FnOnce() -> std::io::Result<bool>,
    ) -> std::io::Result<bool> {
        let state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("flight state lock poisoned"))?;
        if !self.requirement.permits(&state, Instant::now()) {
            return Ok(false);
        }
        operation()
    }
}

#[cfg(test)]
mod tests;

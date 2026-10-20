use std::time::Instant;

use pilotage_mavlink::ardupilot::ArduCopterMode;
use pilotage_mavlink::codec::MAV_CMD_NAV_TAKEOFF;
use pilotage_mavlink::sampling::{OperationalPose, operational_pose};
use pilotage_mavlink::{LandedState, LinkState, RelativeAltitudeReport, command_outcome};
use pilotage_protocol::CommandOutcome;

use super::{
    ArduPilotUplink, FlightPhase, Phase, REPORT_MAX_AGE, Reason, TAKEOFF_ALTITUDE_M,
    TAKEOFF_MARGIN_M, TakeoffProcedure,
};

pub(super) fn start_observation(
    state: &LinkState,
    now: Instant,
) -> Result<(RelativeAltitudeReport, f32), &'static str> {
    if !heartbeat_fresh(state, now) || state.heartbeat_armed != Some(true) {
        return Err("takeoff needs a fresh armed heartbeat");
    }
    if !guided(state) {
        return Err("takeoff needs reported GUIDED mode");
    }
    if state
        .landed_state_cached(now, REPORT_MAX_AGE)
        .is_none_or(|report| report.state != LandedState::OnGround)
    {
        return Err("takeoff needs a fresh on-ground report");
    }
    let pose = authorized_pose(state, now)
        .filter(|pose| {
            pose.yaw_rad.is_finite() && pose.pos_ned_m.iter().all(|value| value.is_finite())
        })
        .ok_or("takeoff needs an authorized finite position")?;
    let altitude = state
        .relative_altitude_cached(now, REPORT_MAX_AGE)
        .ok_or("takeoff needs fresh home-relative altitude")?;
    if !altitude_matches_position(state, altitude) {
        return Err("takeoff altitude and position do not share a current time reference");
    }
    Ok((altitude, pose.pos_ned_m[2]))
}

fn authorized_pose(state: &LinkState, now: Instant) -> Option<OperationalPose> {
    let fresh = |at| {
        now.checked_duration_since(at)
            .is_some_and(|age| age < REPORT_MAX_AGE)
    };
    if !fresh(state.attitude?.received_at) || !fresh(state.kinematics?.received_at) {
        return None;
    }
    operational_pose(state).filter(|pose| {
        pose.yaw_rad.is_finite() && pose.pos_ned_m.iter().all(|value| value.is_finite())
    })
}

fn altitude_matches_position(state: &LinkState, altitude: RelativeAltitudeReport) -> bool {
    state.kinematics.is_some_and(|position| {
        position.stamp.clock == altitude.stamp.clock
            && position
                .stamp
                .acquired_at_ns
                .abs_diff(altitude.stamp.acquired_at_ns)
                <= u64::from(state.maximum_inter_group_skew_ms).saturating_mul(1_000_000)
    })
}

fn heartbeat_fresh(state: &LinkState, now: Instant) -> bool {
    state
        .last_heartbeat
        .and_then(|at| now.checked_duration_since(at))
        .is_some_and(|age| age < REPORT_MAX_AGE)
}

fn guided(state: &LinkState) -> bool {
    state.fc_reports.custom_mode == Some(ArduCopterMode::Guided.custom_mode())
}

impl ArduPilotUplink {
    /// Advances from cached receive facts without renewing their ages.
    pub(crate) fn observe_takeoff(&mut self, state: &LinkState, now: Instant) {
        let Some(procedure) = self.takeoff.as_mut() else {
            return;
        };
        if heartbeat_fresh(state, now)
            && state.heartbeat_armed == Some(false)
            && state.last_heartbeat.is_some_and(|at| {
                at > if procedure.active() {
                    procedure.requested_at
                } else {
                    procedure.status.changed_at
                }
            })
        {
            if procedure.active() {
                procedure.transition(Phase::Cancelled, Some(Reason::VehicleDisarmed), now);
            }
            procedure.blocks_motion = false;
            self.phase = FlightPhase::Ground;
            self.velocity.send_replace(None);
        } else if procedure.active() {
            procedure.observe(state, now);
            if procedure.status.phase == Phase::Completed {
                procedure.blocks_motion = false;
                self.phase = FlightPhase::Flying;
            }
        }
        self.takeoff_revision = procedure.status.revision;
    }
}

impl TakeoffProcedure {
    fn observe(&mut self, state: &LinkState, now: Instant) {
        if self.source_epoch != state.source_epoch
            || self.source_incarnation != state.source_incarnation
            || self.source_id != state.source_id
        {
            self.transition(Phase::Failed, Some(Reason::SourceChanged), now);
            return;
        }
        if now >= self.complete_before {
            self.transition(Phase::TimedOut, Some(Reason::DeadlineExpired), now);
            return;
        }
        if !heartbeat_fresh(state, now) || !self.reference_is_current(state, now) {
            self.transition(Phase::Failed, Some(Reason::MeasurementUnavailable), now);
            return;
        }
        if !guided(state) {
            self.transition(Phase::Failed, Some(Reason::ModeChanged), now);
            return;
        }
        let Some(written_at) = self.receipt.sent_at_cached() else {
            if now >= self.send_before {
                self.transition(Phase::TimedOut, Some(Reason::DeadlineExpired), now);
            }
            return;
        };
        if self.status.phase == Phase::Queued {
            self.transition(Phase::Written, None, now);
        }
        if let Some(ack) = state
            .fc_reports
            .command_ack(MAV_CMD_NAV_TAKEOFF)
            .filter(|ack| ack.received_at >= written_at)
        {
            let outcome = command_outcome(ack.result);
            self.acknowledge(outcome, now);
            if !matches!(
                outcome,
                CommandOutcome::Accepted | CommandOutcome::InProgress
            ) {
                self.transition(Phase::Failed, Some(Reason::CommandRefused), now);
                return;
            }
            self.transition(Phase::InProgress, None, now);
        }
        if self.climb_observed(state, now, written_at) {
            self.transition(Phase::Completed, None, now);
        }
    }

    fn reference_is_current(&self, state: &LinkState, now: Instant) -> bool {
        let Some(pose) = authorized_pose(state, now) else {
            return false;
        };
        let Some(altitude) = state.relative_altitude_cached(now, REPORT_MAX_AGE) else {
            return false;
        };
        altitude_matches_position(state, altitude)
            && (altitude.altitude_m + pose.pos_ned_m[2] - self.home_offset_m).abs()
                <= TAKEOFF_MARGIN_M
            && state.landed_state_cached(now, REPORT_MAX_AGE).is_some()
    }

    fn climb_observed(&self, state: &LinkState, now: Instant, written_at: Instant) -> bool {
        let Some(altitude) = state.relative_altitude_cached(now, REPORT_MAX_AGE) else {
            return false;
        };
        let Some(landed) = state.landed_state_cached(now, REPORT_MAX_AGE) else {
            return false;
        };
        let Some(pose) = authorized_pose(state, now) else {
            return false;
        };
        let fresh_position = state.kinematics.is_some_and(|position| {
            position.received_at > written_at
                && position.stamp.source_epoch == self.source_epoch
                && position.stamp.source_incarnation == self.source_incarnation
                && position.stamp.source_id == self.source_id
                && position.stamp.clock == self.baseline_position_stamp.clock
                && position.stamp.acquired_at_ns > self.baseline_position_stamp.acquired_at_ns
        });
        fresh_position
            && altitude.received_at > written_at
            && altitude.time_boot_ms > self.baseline_time_boot_ms
            && landed.received_at > written_at
            && landed.state == LandedState::InAir
            && altitude.altitude_m >= self.target_altitude_m - TAKEOFF_MARGIN_M
            && self.start_down_m - pose.pos_ned_m[2] >= TAKEOFF_ALTITUDE_M - TAKEOFF_MARGIN_M
    }
}

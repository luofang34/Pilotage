//! The end of a plan. A vehicle that can stop flies on to the last
//! waypoint, so the plan does not complete a capture radius short of it,
//! and is then commanded to zero velocity. A fixed-wing aircraft cannot
//! stop: its plan completes when the last waypoint sequences, and with no
//! further instruction it then flies a standard holding pattern at the
//! last waypoint (see [`crate::holding_pattern`]). It is never left
//! without a command: silence is its flight controller's command-loss
//! descent.

use navigate_contract::{GeodeticPosition, MonotonicNanos, NavigationSolution};
use navigate_route::SequenceReason;
use pilotage_protocol::{ControlIntent, ReferenceFrame, VelocityIntent};

use super::super::{MissionEngine, MissionEvent, MissionOutput, MissionState};
use super::outputs::zero_velocity_intent;

impl MissionEngine {
    /// The output of a tick of a vehicle flying on after its plan.
    pub(super) fn after_plan_output(&mut self, now: MonotonicNanos) -> MissionOutput {
        let mut events = core::mem::take(&mut self.hold_events);
        let solution = self.publish_solution(now);
        let state = MissionState::AfterPlan;
        let intent = self.intent_for_state(state, solution.as_ref(), now, &mut events);
        MissionOutput {
            intent,
            action: None,
            state,
            events,
        }
    }

    /// The intent after the plan completed, by vehicle class.
    pub(in crate::engine) fn end_of_plan_intent(
        &mut self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        if self.config.can_stop() {
            return Some(zero_velocity_intent());
        }
        let guided = solution.and_then(|solution| {
            let (reference, target) = self.holding_pattern_target(solution, now, events)?;
            self.guide_to(solution, reference, &target, now, events)
        });
        guided.or_else(|| self.holding_pattern_fallback())
    }

    /// The command of a vehicle in its holding pattern when guidance gives
    /// none: the heading and altitude last commanded for a fixed wing
    /// steered by heading; for one steered by velocity, straight and level
    /// on its present heading at the cruise airspeed.
    fn holding_pattern_fallback(&mut self) -> Option<ControlIntent> {
        if self.steers_by_heading() {
            let cruise_altitude_m = self.config.anchor.altitude_m + self.config.cruise_height_m;
            let altitude_m = self
                .holding_pattern()
                .map_or(cruise_altitude_m, |status| status.pattern.fix.altitude_m);
            return self.repeat_heading(altitude_m);
        }
        Some(ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx: self.commanded_cruise_mps() as f32,
            vy: 0.0,
            vz: 0.0,
            yaw_rate: 0.0,
        }))
    }

    /// Records the sequence of the last waypoint. A vehicle that can stop
    /// flies on to arrive at it; one that cannot completes the plan here,
    /// and its holding pattern starts with the next intent.
    pub(super) fn last_waypoint_sequenced(
        &mut self,
        position: &GeodeticPosition,
        reason: SequenceReason,
        events: &mut Vec<MissionEvent>,
    ) {
        let distance_m = self.route.distance_to_last_m(position).unwrap_or(f64::NAN);
        events.push(MissionEvent::LastWaypointSequenced { reason, distance_m });
        self.plan_complete = !self.route.approaching();
    }
}

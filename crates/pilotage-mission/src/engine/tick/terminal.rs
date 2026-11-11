//! The end of a plan. A vehicle that can stop flies on to the last
//! waypoint, so the plan does not complete a capture radius short of it,
//! and is then commanded to zero velocity. A fixed-wing aircraft cannot
//! stop: its plan completes when the last waypoint sequences, and it then
//! orbits the last waypoint clockwise.

use navigate_contract::{
    GeodeticPosition, LateralReference, MonotonicNanos, NavigationSolution, Waypoint,
};
use navigate_geodesy::NedOffset;
use navigate_route::SequenceReason;
use pilotage_protocol::ControlIntent;

use crate::body;

use super::super::{MissionEngine, MissionEvent, MissionOutput, MissionState};
use super::outputs::{terminal_output, zero_velocity_intent};

/// How far ahead of the vehicle's bearing from the centre the guided
/// point lies, radians. Far enough that the guidance turns smoothly, near
/// enough that the path stays close to the circle.
const ORBIT_LEAD_RAD: f64 = core::f64::consts::FRAC_PI_4;

impl MissionEngine {
    /// The output of a tick after the mission completed.
    pub(super) fn complete_output(&mut self, now: MonotonicNanos) -> MissionOutput {
        if self.config.can_stop() {
            return terminal_output(MissionState::Complete);
        }
        let mut events = core::mem::take(&mut self.hold_events);
        let solution = self.publish_solution(now);
        let intent = self.end_of_plan_intent(solution.as_ref(), now, &mut events);
        MissionOutput {
            intent,
            action: None,
            state: MissionState::Complete,
            events,
        }
    }

    /// The intent after the plan completed, by vehicle class.
    pub(super) fn end_of_plan_intent(
        &mut self,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        if self.config.can_stop() {
            return Some(zero_velocity_intent());
        }
        let solution = solution?;
        let (centre, radius_m) = self.route.terminal_orbit()?;
        let lead = orbit_lead_point(solution, &centre, radius_m)?;
        self.guide_to(
            solution,
            LateralReference::PresentPosition,
            &lead,
            now,
            events,
        )
    }

    /// Records the sequence of the last waypoint. A vehicle that can stop
    /// flies on to arrive at it; one that cannot completes the plan here
    /// and starts its orbit.
    pub(super) fn last_waypoint_sequenced(
        &mut self,
        position: &GeodeticPosition,
        reason: SequenceReason,
        events: &mut Vec<MissionEvent>,
    ) {
        let distance_m = self.route.distance_to_last_m(position).unwrap_or(f64::NAN);
        events.push(MissionEvent::LastWaypointSequenced { reason, distance_m });
        self.plan_complete = !self.route.approaching();
        if !self.plan_complete || self.config.can_stop() {
            return;
        }
        if let Some((centre, radius_m)) = self.route.terminal_orbit() {
            events.push(MissionEvent::OrbitStarted {
                radius_m,
                altitude_m: centre.position.altitude_m,
            });
        }
    }
}

/// The guided point for a clockwise orbit of `radius_m` around `centre`,
/// at the centre's altitude: [`ORBIT_LEAD_RAD`] clockwise of the vehicle's
/// bearing from the centre, at `radius_m / cos(lead)` from it. A vehicle
/// at radius `r` that flies toward that point moves square to its bearing
/// only when `r = radius_m`, so the steady path is the orbit itself and
/// not a tighter circle than the turn radius allows.
fn orbit_lead_point(
    solution: &NavigationSolution,
    centre: &Waypoint,
    radius_m: f64,
) -> Option<Waypoint> {
    let plane = body::tangent_plane(centre.position).ok()?;
    let offset = plane.to_ned(&solution.position);
    let bearing = offset.east_m.atan2(offset.north_m) + ORBIT_LEAD_RAD;
    let lead_m = radius_m / ORBIT_LEAD_RAD.cos();
    let mut position = plane.from_ned(&NedOffset::new(
        lead_m * bearing.cos(),
        lead_m * bearing.sin(),
        0.0,
    ));
    position.altitude_m = centre.position.altitude_m;
    let mut lead = *centre;
    lead.position = position;
    Some(lead)
}

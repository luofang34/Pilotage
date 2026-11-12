//! Flying a lateral mode: the intent each mode gives.

use navigate_contract::{MonotonicNanos, NavigationSolution};
use pilotage_mission_core::FlightAction as CoreFlightAction;
use pilotage_protocol::ControlIntent;

use super::{LateralEvent, LateralMode};
use crate::engine::{MissionEngine, MissionEvent, MissionState};

impl MissionEngine {
    /// The intent of `mode`, entered on `event`, in `state`.
    ///
    /// The mission's phase still chooses within two modes, because it is
    /// what sets the vertical part of their intent: a vehicle that stays
    /// where it is climbs in its climb and stops after its plan, and a
    /// route flown in the climb is flown at the climb's target altitude.
    pub(super) fn fly_lateral(
        &mut self,
        mode: LateralMode,
        event: LateralEvent,
        state: MissionState,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let cruise_altitude_m = self.config.anchor.altitude_m + self.config.cruise_height_m;
        match mode {
            LateralMode::Off => None,
            LateralMode::Standby => self.standby_intent(solution),
            LateralMode::PositionHold => match event {
                LateralEvent::Lost(_) => self.hold_position(now, events),
                _ if state == MissionState::Climb => self.climb_intent(solution),
                _ => self.end_of_plan_intent(solution, now, events),
            },
            LateralMode::Pattern if state == MissionState::AfterPlan => {
                self.end_of_plan_intent(solution, now, events)
            }
            LateralMode::DirectTo | LateralMode::Track | LateralMode::Pattern => {
                self.route_intent(state, solution, now, events)
            }
            LateralMode::HeadingHold => self.hold_heading(cruise_altitude_m, events),
            // The pattern flown goes on by time; with none, a racetrack
            // begins where the vehicle is; with no heading to begin one
            // from, the last command stands.
            LateralMode::TimedHold => self
                .timed_holding_intent(now, events)
                .or_else(|| self.racetrack_intent(now, events))
                .or_else(|| self.hold_heading(cruise_altitude_m, events)),
            LateralMode::Coast => self.end_of_plan_intent(None, now, events),
        }
    }

    /// The intent from the arm request until flight: for a vehicle
    /// steered by velocity, the climb that follows the arm or the hold of
    /// where it is; for one steered by heading, its present heading at
    /// the cruise altitude, or at its own altitude with no climb to fly.
    fn standby_intent(&mut self, solution: Option<&NavigationSolution>) -> Option<ControlIntent> {
        if !self.steers_by_heading() {
            return self.arming_intent(solution);
        }
        let altitude_m = if self.config.cruise_height_m > 0.0 {
            self.config.anchor.altitude_m + self.config.cruise_height_m
        } else {
            solution?.position.altitude_m
        };
        self.steer_level(altitude_m)
    }

    /// The intent of the route: the plan's guidance, flown in the climb
    /// of a vehicle that cannot stop with the climb as its vertical part.
    /// A vehicle steered by heading flies the route at the climb's target
    /// altitude, and on its present heading while there is no guidance.
    fn route_intent(
        &mut self,
        state: MissionState,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        if state != MissionState::Climb {
            return self.follow_plan_intent(solution, now, events);
        }
        if !self.steers_by_heading() {
            return self.climb_along_route_intent(solution, now, events);
        }
        let Some(CoreFlightAction::Climb { target_altitude_m }) = self.active_action.as_ref()
        else {
            return None;
        };
        let target = *target_altitude_m;
        match self.follow_plan_intent(solution, now, events) {
            Some(ControlIntent::HeadingAltitude(demand)) => {
                Some(self.heading_altitude(f64::from(demand.heading_rad), target))
            }
            _ => self.steer_level(target),
        }
    }
}

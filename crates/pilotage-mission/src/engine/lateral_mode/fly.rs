//! Flying a lateral mode: the intent each mode gives.

use navigate_contract::{MonotonicNanos, NavigationSolution};
use pilotage_mission_core::FlightAction as CoreFlightAction;
use pilotage_protocol::ControlIntent;

use super::super::lifecycle::LateralStage;
use super::{LateralEvent, LateralMode};
use crate::engine::{MissionEngine, MissionEvent};

impl MissionEngine {
    /// The intent of `mode`, entered on `event`, in `stage`.
    pub(super) fn fly_lateral(
        &mut self,
        mode: LateralMode,
        event: LateralEvent,
        stage: LateralStage,
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
                _ if stage == LateralStage::Climb => self.climb_intent(solution),
                _ => self.end_of_plan_intent(solution, now, events),
            },
            LateralMode::Pattern if stage == LateralStage::AfterPlan => {
                self.end_of_plan_intent(solution, now, events)
            }
            LateralMode::DirectTo | LateralMode::Track | LateralMode::Pattern => {
                self.route_intent(stage, solution, now, events)
            }
            LateralMode::HeadingHold => self.hold_heading(cruise_altitude_m, events),
            // The pattern flown goes on by time; with none, a racetrack
            // begins where the vehicle is; with no heading to begin one
            // from, the last command stands.
            LateralMode::TimedHold => self
                .timed_holding_intent(now, events)
                .or_else(|| self.racetrack_intent(now, events))
                .or_else(|| self.hold_heading(cruise_altitude_m, events)),
        }
    }

    /// The intent from the arm request until flight.
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

    /// The intent of the route.
    fn route_intent(
        &mut self,
        stage: LateralStage,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        if stage != LateralStage::Climb {
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

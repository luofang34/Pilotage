//! The lateral mode machine: what the vehicle flies over the ground.
//!
//! One pure function, [`next`], turns a mode and one event into the next
//! mode and what to act on. The events come from outside: the plan and
//! the mission say what is demanded ([`Demand`]), the navigation source
//! says when there is no position to fly it on, and the vehicle's
//! capability says how that loss is answered ([`LossResponse`]). The
//! machine reads none of them itself, so no lateral behaviour is implied
//! by another machine's state.
//!
//! A vehicle that cannot stop never flies a straight line without end on
//! a lost position: its heading hold is bounded by a timer, and when the
//! timer runs out it flies a racetrack by time.
//!
//! The machine does not select the intent yet. Each tick steps it on the
//! plan's demand, and the engine's own selection records the mode it
//! flew; a tick on which the two differ is counted. The timer is not run
//! until the machine selects the intent.

use navigate_contract::{GeodeticPosition, MonotonicNanos, NavigationSolution};
use pilotage_protocol::ControlIntent;

use super::tick::overflown;
use super::{MissionEngine, MissionEvent, MissionState};

#[cfg(test)]
mod tests;

/// The lateral mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) enum LateralMode {
    /// Nothing is flown.
    #[default]
    Off,
    /// The present heading or position is kept from the arm request until
    /// flight.
    Standby,
    /// The vehicle stays where it is.
    PositionHold,
    /// Direct to a fix from the present position.
    DirectTo,
    /// Along the track of a leg to its fix.
    Track,
    /// The holding pattern after the plan, flown on position.
    Pattern,
    /// The heading last commanded is held, with no position to fly on,
    /// for a bounded time.
    HeadingHold,
    /// A racetrack flown by heading and time, with no position to fly on.
    TimedHold,
    /// Straight and level on the present heading, with no position to fly
    /// on and no heading command to fly a racetrack with.
    Coast,
}

/// What the plan and the mission ask of the lateral mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Demand {
    /// Keep the present heading or position until flight.
    Standby,
    /// Stay here.
    Position,
    /// Fly direct to a fix.
    DirectTo,
    /// Fly a leg's track.
    Track,
    /// Fly the holding pattern after the plan.
    Pattern,
}

/// How a vehicle answers a demand it has no position to fly on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LossResponse {
    /// It can stop, and its flight controller's own position is valid:
    /// it stays where it is.
    HoldPosition,
    /// It can stop, and its flight controller has no valid position: it
    /// is sent nothing, and the flight controller's command-loss action
    /// takes over.
    Silent,
    /// It cannot stop and is steered by heading: it holds its heading,
    /// then flies a racetrack by time.
    HoldHeading,
    /// It cannot stop and is steered by velocity: it flies on straight
    /// and level.
    Coast,
}

/// What one tick asks of the lateral mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LateralEvent {
    /// Nothing is flown.
    Disengaged,
    /// The demand, with a position to fly it on or no need of one.
    Demand(Demand),
    /// The demand needs a position and there is none.
    Lost(LossResponse),
    /// The bound of the heading hold ran out.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the engine runs no timer while it only observes the machine"
        )
    )]
    TimerExpired,
}

/// What a transition gives to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LateralEffect {
    /// Nothing.
    None,
    /// The heading hold began: its timer starts.
    HeadingHoldBegan,
    /// The timed racetrack began.
    TimedHoldBegan,
    /// A position came back during the heading hold or the timed
    /// racetrack: the timer is cleared and the mode now flown is resumed.
    Resumed,
}

/// The mode after `event` in `mode`, and what to act on.
pub(super) const fn next(mode: LateralMode, event: LateralEvent) -> (LateralMode, LateralEffect) {
    use LateralMode::{
        Coast, DirectTo, HeadingHold, Off, Pattern, PositionHold, Standby, TimedHold, Track,
    };
    let to = match (mode, event) {
        (_, LateralEvent::Disengaged | LateralEvent::Lost(LossResponse::Silent)) => Off,
        (_, LateralEvent::Demand(Demand::Standby)) => Standby,
        (_, LateralEvent::Demand(Demand::Position)) => PositionHold,
        (_, LateralEvent::Demand(Demand::DirectTo)) => DirectTo,
        (_, LateralEvent::Demand(Demand::Track)) => Track,
        (_, LateralEvent::Demand(Demand::Pattern)) => Pattern,
        (_, LateralEvent::Lost(LossResponse::HoldPosition)) => PositionHold,
        (_, LateralEvent::Lost(LossResponse::Coast)) => Coast,
        // A pattern that loses its position goes on by time at once: it
        // already circles a fix.
        (Pattern | TimedHold, LateralEvent::Lost(LossResponse::HoldHeading)) => TimedHold,
        (_, LateralEvent::Lost(LossResponse::HoldHeading)) => HeadingHold,
        (HeadingHold, LateralEvent::TimerExpired) => TimedHold,
        (other, LateralEvent::TimerExpired) => other,
    };
    let effect = match (mode, to) {
        (HeadingHold, HeadingHold) | (TimedHold, TimedHold) => LateralEffect::None,
        (_, HeadingHold) => LateralEffect::HeadingHoldBegan,
        (_, TimedHold) => LateralEffect::TimedHoldBegan,
        (HeadingHold | TimedHold, Standby | PositionHold | DirectTo | Track | Pattern)
            if matches!(event, LateralEvent::Demand(_)) =>
        {
            LateralEffect::Resumed
        }
        _ => LateralEffect::None,
    };
    (to, effect)
}

/// The machine's mode beside the mode the engine flew.
#[derive(Debug, Default)]
pub(super) struct Lateral {
    /// The machine's mode after the latest tick.
    pub(super) mode: LateralMode,
    /// The mode the engine's own selection flew on the latest tick.
    pub(super) flown: LateralMode,
}

impl MissionEngine {
    /// The intent of a tick in `state`. Steps the lateral machine on what
    /// the plan demands, lets the engine select its intent, and counts a
    /// tick on which the mode selected is not the machine's
    /// ([`super::MissionCounters::lateral_mode_mismatch`]).
    pub(super) fn intent_for_state(
        &mut self,
        state: MissionState,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let event = self.lateral_event(state, solution, now);
        let (mode, _) = next(self.modes.lateral.mode, event);
        self.modes.lateral = Lateral {
            mode,
            flown: LateralMode::Off,
        };
        let intent = self.select_intent(state, solution, now, events);
        if self.modes.lateral.flown != mode {
            self.counters.lateral_mode_mismatch =
                self.counters.lateral_mode_mismatch.wrapping_add(1);
        }
        intent
    }

    /// Records the lateral mode the engine's selection flies this tick.
    pub(super) const fn fly_lateral(&mut self, mode: LateralMode) {
        self.modes.lateral.flown = mode;
    }

    /// What this tick asks of the lateral mode. The mission's phase is
    /// read here once, to hand the machine its demand.
    fn lateral_event(
        &self,
        state: MissionState,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
    ) -> LateralEvent {
        let by_heading = self.steers_by_heading();
        let can_stop = self.config.can_stop();
        match state {
            MissionState::Arming if self.arm_request_sent() => {
                LateralEvent::Demand(Demand::Standby)
            }
            // Launched with no solution fit to fly.
            MissionState::AwaitSolution
                if by_heading && solution.is_none() && self.launched(now) =>
            {
                LateralEvent::Lost(LossResponse::HoldHeading)
            }
            MissionState::Climb | MissionState::Enroute | MissionState::Complete => {
                let Some(solution) = solution else {
                    return LateralEvent::Lost(self.loss_response(state, now));
                };
                let demand = match state {
                    MissionState::Climb if can_stop => Some(Demand::Position),
                    MissionState::Climb if by_heading && !self.flies_route() => None,
                    // The tick that completes the plan stops a vehicle that
                    // can stop; one that cannot flies its pattern from then on.
                    MissionState::Complete if can_stop => Some(Demand::Position),
                    MissionState::Complete => Some(Demand::Pattern),
                    _ => self.route_demand(&solution.position),
                };
                demand.map_or(LateralEvent::Disengaged, LateralEvent::Demand)
            }
            MissionState::AwaitSolution
            | MissionState::Arming
            | MissionState::Held
            | MissionState::Failed => LateralEvent::Disengaged,
        }
    }

    /// How this vehicle answers a demand it has no position to fly on.
    fn loss_response(&self, state: MissionState, now: MonotonicNanos) -> LossResponse {
        if self.steers_by_heading() {
            LossResponse::HoldHeading
        } else if state == MissionState::Complete {
            LossResponse::Coast
        } else if self.fc_position_valid(now) {
            LossResponse::HoldPosition
        } else {
            LossResponse::Silent
        }
    }

    /// What the plan demands of a vehicle at `position`: nothing while
    /// its path is not flown; after its last waypoint, the stop of a
    /// vehicle that can stop and the holding pattern of one that cannot;
    /// direct to the last waypoint a stopping vehicle flies on to, to a
    /// first fix with no leg behind it, and back to a fix flown past; and
    /// otherwise the active leg's track.
    fn route_demand(&self, position: &GeodeticPosition) -> Option<Demand> {
        if !self.flies_route() {
            return None;
        }
        if self.plan_complete {
            return Some(if self.config.can_stop() {
                Demand::Position
            } else {
                Demand::Pattern
            });
        }
        if self.route.approaching() {
            return Some(Demand::DirectTo);
        }
        let leg = self.route.active_leg()?;
        Some(match leg.from {
            Some(from) if !overflown(position, &from.position, &leg.to.position) => Demand::Track,
            _ => Demand::DirectTo,
        })
    }
}

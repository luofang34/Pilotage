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
//! Each tick steps the machine on the plan's demand and flies the mode it
//! gives ([`MissionEngine::intent_for_state`]).

use navigate_contract::{GeodeticPosition, MonotonicNanos, NavigationSolution};
use pilotage_protocol::ControlIntent;

use super::tick::overflown;
use super::{MissionEngine, MissionEvent, MissionState};

use crate::holding_pattern::TimedCircuit;

mod fly;
#[cfg(test)]
mod tests;

/// The lateral mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LateralMode {
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

/// The lateral mode as a display reads it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LateralStatus {
    /// The mode flown.
    pub mode: LateralMode,
    /// A racetrack flown by time is corrected with a wind estimate older
    /// than the configured age: where it lies over the ground is an
    /// assumption.
    pub wind_stale: bool,
}

/// The machine's mode with what its holds keep between ticks.
#[derive(Debug, Default)]
pub(super) struct Lateral {
    /// The mode after the latest tick.
    pub(super) mode: LateralMode,
    /// Since when the heading has been held, while it is.
    held_since: Option<MonotonicNanos>,
    /// The racetrack flown by time with no pattern to take it from.
    pub(super) racetrack: Option<TimedCircuit>,
    /// Whether the racetrack's wind estimate is stale.
    pub(super) wind_stale: bool,
}

impl MissionEngine {
    /// The lateral mode flown, from the state the latest
    /// [`MissionEngine::tick`] left.
    #[must_use]
    pub const fn lateral_status(&self) -> LateralStatus {
        LateralStatus {
            mode: self.modes.lateral.mode,
            wind_stale: self.modes.lateral.wind_stale,
        }
    }

    /// The intent of a tick in `state`: steps the lateral machine on what
    /// the plan demands, runs the heading hold's timer, and flies the
    /// mode the machine gives.
    pub(super) fn intent_for_state(
        &mut self,
        state: MissionState,
        solution: Option<&NavigationSolution>,
        now: MonotonicNanos,
        events: &mut Vec<MissionEvent>,
    ) -> Option<ControlIntent> {
        let event = self.lateral_event(state, solution, now);
        let (mut mode, mut effect) = next(self.modes.lateral.mode, event);
        if mode == LateralMode::HeadingHold {
            let since = *self.modes.lateral.held_since.get_or_insert(now);
            let bound = self.config.heading_hold_bound;
            let expired = now
                .elapsed_since(since)
                .is_some_and(|held| held.as_nanos() >= bound.as_nanos());
            if expired {
                (mode, effect) = next(mode, LateralEvent::TimerExpired);
            }
        }
        if mode != LateralMode::HeadingHold {
            self.modes.lateral.held_since = None;
        }
        if mode != LateralMode::TimedHold {
            self.modes.lateral.racetrack = None;
            self.modes.lateral.wind_stale = false;
        }
        self.modes.lateral.mode = mode;
        if effect == LateralEffect::Resumed {
            events.push(MissionEvent::LateralResumed { mode });
        }
        self.fly_lateral(mode, event, state, solution, now, events)
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

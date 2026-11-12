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
        _ => LateralEffect::None,
    };
    (to, effect)
}

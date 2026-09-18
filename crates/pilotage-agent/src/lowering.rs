//! The lowering of a directive onto the flight actions of the mission core
//! (ADR-0041, ADR-0042).
//!
//! The directive vocabulary is not a second mission vocabulary. Each directive
//! that the agent can fly has a sequence of mission flight actions, and this
//! module is the one place that states it. The match is exhaustive, so a new
//! directive kind does not compile until it has a lowering.
//!
//! The executor of this crate does not run these actions today. The mission
//! document does not change after a mission starts, and a directive changes
//! the active target in flight. The lowering keeps the two vocabularies
//! together until the mission core can take a directive in flight.

use pilotage_mission_core::{FlightAction, FlightPlanReference, TurnDirection as MissionTurn};

use crate::directive::{Arrival, Directive, HoldPoint, TurnDirection};
use crate::scenario::HOME;

/// What a lowering needs from outside the directive.
pub struct LoweringContext<'a> {
    /// The altitude of the launch point in metres, in the altitude reference
    /// of the mission document.
    pub launch_altitude_m: f64,
    /// The height above the launch point for a takeoff and a go-around.
    pub cruise_height_m: f64,
    /// Gives the immutable flight plan to a fix or along a procedure. The
    /// flight-planning module of a client owns these plans.
    pub plan_for: &'a dyn Fn(&str) -> Option<FlightPlanReference>,
}

/// Why a directive has no lowering.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoweringError {
    /// No flight plan goes to this fix or along this procedure.
    #[error("no flight plan is known for {name}")]
    NoPlan {
        /// The fix name or the procedure name.
        name: String,
    },
}

/// The mission flight actions for one directive, in sequence. `unable` has
/// none.
pub fn flight_actions(
    directive: &Directive,
    context: &LoweringContext<'_>,
) -> Result<Vec<FlightAction>, LoweringError> {
    let cruise_altitude_m = context.launch_altitude_m + context.cruise_height_m;
    let follow = |name: &str| {
        (context.plan_for)(name)
            .map(|plan| FlightAction::FollowPlan { plan })
            .ok_or_else(|| LoweringError::NoPlan {
                name: name.to_owned(),
            })
    };
    let at_arrival = |arrival: Arrival| match arrival {
        Arrival::Land => FlightAction::Land {},
        Arrival::Hold => FlightAction::MaintainTarget {},
    };
    Ok(match directive {
        Directive::Takeoff {} => vec![
            FlightAction::Arm {},
            FlightAction::Climb {
                target_altitude_m: cruise_altitude_m,
            },
        ],
        Directive::DirectTo { fix, on_arrival } => vec![follow(fix)?, at_arrival(*on_arrival)],
        Directive::Heading { degrees, turn } => vec![FlightAction::Heading {
            true_heading_rad: f64::from(*degrees).to_radians(),
            turn: match turn {
                TurnDirection::Left => MissionTurn::Left,
                TurnDirection::Right => MissionTurn::Right,
                TurnDirection::Shortest => MissionTurn::Shortest,
            },
        }],
        Directive::Altitude { height_m } => vec![FlightAction::Climb {
            target_altitude_m: context.launch_altitude_m + height_m,
        }],
        Directive::Speed { speed_mps } => vec![FlightAction::Speed {
            speed_mps: *speed_mps,
        }],
        Directive::Hold { point } => match point {
            HoldPoint::PresentPosition => vec![FlightAction::MaintainTarget {}],
            HoldPoint::Fix(fix) => vec![follow(fix)?, FlightAction::MaintainTarget {}],
        },
        // The procedure states what the vehicle does at its last fix, and the
        // plan of the procedure ends there. The arrival action is thus part
        // of the plan owner's answer and not of this lowering.
        Directive::JoinProcedure { procedure } => vec![follow(procedure)?],
        Directive::Land {} => vec![FlightAction::Land {}],
        Directive::GoAround {} => vec![FlightAction::GoAround {
            target_altitude_m: cruise_altitude_m,
        }],
        Directive::ReturnToBase {} => vec![follow(HOME)?, FlightAction::Land {}],
        Directive::Unable { .. } => Vec::new(),
    })
}

#[cfg(test)]
mod tests;

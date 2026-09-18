//! The deterministic flight sequencer.
//!
//! The classifier supplies a destination and an arrival behaviour and
//! nothing else. Every decision that depends on vehicle state is made
//! here, in code that has no language model in it: when to arm, when the
//! climb is complete, when the vehicle has arrived, when it is on the
//! ground. The sequencer has no I/O and no clock of its own, so each
//! decision can be tested.

use pilotage_client_session::MotionDemand;

use crate::guidance;
use crate::scenario::{Arrival, Waypoint};
use crate::vehicle_state::VehicleState;

/// Climb demand from the ground to cruise height. It is well above a
/// hover demand, so a flight controller that detects takeoff from the
/// size of the demand sees one.
const CLIMB_DEMAND: f32 = 0.8;
/// Descent demand for the landing.
const DESCENT_DEMAND: f32 = -0.4;
/// Height below cruise at which the climb counts as complete, in metres.
const CLIMB_CAPTURE_M: f64 = 0.5;
/// Ground speed above which a vehicle inside the arrival radius is still
/// passing through and has not arrived, in metres per second.
const ARRIVAL_SPEED_MPS: f64 = 0.5;
/// Height at or below which the vehicle counts as on the ground.
const GROUND_HEIGHT_M: f64 = 0.3;
/// Height below which a vehicle that stopped descending is on the
/// ground. It covers an estimate that does not read zero at touchdown.
const SETTLED_BELOW_M: f64 = 1.5;
/// Vertical speed below which the vehicle is not descending.
const SETTLED_CLIMB_MPS: f64 = 0.15;
/// Time without descent that confirms touchdown, in seconds.
const SETTLED_FOR_S: f64 = 2.0;
/// Interval between repeats of an arm or disarm request, in seconds.
const ACTION_RETRY_S: f64 = 2.0;

/// Where the flight is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    /// No intent to fly.
    AwaitIntent,
    /// An arm request is out.
    Arming,
    /// Climbing to cruise height.
    Climb,
    /// Flying to the destination.
    Enroute,
    /// At the destination, at cruise height.
    Holding,
    /// Descending at the destination.
    Descending,
    /// On the ground; a disarm request is out.
    Disarming,
    /// On the ground, and disarmed where the vehicle offers a disarm.
    Landed,
}

/// A discrete request for the reliable action channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Discrete {
    /// Arm the vehicle.
    Arm,
    /// Disarm the vehicle.
    Disarm,
}

/// A destination the sequencer can fly to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Destination {
    /// Waypoint name, for the run record.
    pub name: String,
    /// Waypoint position.
    pub waypoint: Waypoint,
    /// Behaviour at the waypoint.
    pub on_arrival: Arrival,
}

/// What the scenario and the host's advertisement fix for one flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlightLimits {
    /// Height above the launch point for the cruise, in metres.
    pub cruise_height_m: f64,
    /// Horizontal distance that counts as arrival, in metres.
    pub arrival_radius_m: f64,
    /// Advertised speed at full horizontal demand, in metres per second.
    pub max_linear_mps: f64,
    /// True when the motion scope advertises a disarm. A vehicle with no
    /// disarm to offer is landed when it is down.
    pub disarm_offered: bool,
}

/// The output of one sequencer step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Step {
    /// The motion demand for this frame.
    pub demand: MotionDemand,
    /// A discrete request to send now, if one is due.
    pub action: Option<Discrete>,
    /// The phase after this step.
    pub phase: Phase,
}

/// The flight state machine.
#[derive(Debug)]
pub(crate) struct Sequencer {
    limits: FlightLimits,
    phase: Phase,
    destination: Option<Destination>,
    /// False from a new destination until the flight to it is complete.
    served: bool,
    acknowledged_armed: bool,
    last_request_s: Option<f64>,
    settled_since_s: Option<f64>,
    enroute_since_s: Option<f64>,
}

impl Sequencer {
    /// A sequencer on the ground with no intent.
    pub(crate) fn new(limits: FlightLimits) -> Self {
        Self {
            limits,
            phase: Phase::AwaitIntent,
            destination: None,
            served: true,
            acknowledged_armed: false,
            last_request_s: None,
            settled_since_s: None,
            enroute_since_s: None,
        }
    }

    /// The current phase.
    #[cfg(test)]
    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    /// Seconds the vehicle has been en route without a break.
    pub(crate) fn enroute_seconds(&self, now_s: f64) -> Option<f64> {
        self.enroute_since_s.map(|since| now_s - since)
    }

    /// Replaces the destination. A vehicle in the air turns to it at
    /// once; a landing in progress is abandoned for it. The en-route time
    /// starts again, because it measures the leg to this destination. A
    /// scripted message that waits on it would otherwise ride on the time
    /// of the leg before.
    pub(crate) fn set_destination(&mut self, destination: Destination) {
        self.destination = Some(destination);
        self.served = false;
        if matches!(
            self.phase,
            Phase::Enroute | Phase::Holding | Phase::Descending
        ) {
            self.enter(Phase::Enroute, None);
        }
    }

    /// Records the host's answer to a discrete request.
    pub(crate) fn on_action_result(&mut self, request: Discrete, accepted: bool) {
        if accepted {
            self.acknowledged_armed = request == Discrete::Arm;
        }
    }

    /// Advances the machine by one frame.
    pub(crate) fn step(&mut self, state: &VehicleState, now_s: f64) -> Step {
        // The flight controller's word wins. The acknowledgement serves
        // a vehicle that reports no armed state.
        let armed = state.armed.unwrap_or(self.acknowledged_armed);
        let (demand, action) = match self.phase {
            Phase::AwaitIntent | Phase::Landed => self.on_ground(now_s),
            Phase::Arming => self.arming(state, armed, now_s),
            Phase::Climb => self.climb(state, now_s),
            Phase::Enroute => self.enroute(state, now_s),
            Phase::Holding => self.holding(state),
            Phase::Descending => self.descending(state, now_s),
            Phase::Disarming => self.disarming(armed, now_s),
        };
        Step {
            demand,
            action,
            phase: self.phase,
        }
    }

    fn enter(&mut self, phase: Phase, now_s: Option<f64>) {
        self.phase = phase;
        self.last_request_s = None;
        self.settled_since_s = None;
        self.enroute_since_s = if phase == Phase::Enroute { now_s } else { None };
    }

    /// The demand that keeps the vehicle on its destination with the
    /// given vertical demand, or still when there is no destination.
    fn on_station(&self, state: &VehicleState, throttle: f32) -> MotionDemand {
        self.destination.as_ref().map_or(
            MotionDemand {
                throttle,
                ..guidance::STOP
            },
            |destination| {
                guidance::toward(
                    state,
                    destination.waypoint,
                    self.limits.max_linear_mps,
                    throttle,
                )
            },
        )
    }

    fn on_ground(&mut self, now_s: f64) -> (MotionDemand, Option<Discrete>) {
        if !self.served && self.destination.is_some() {
            self.enter(Phase::Arming, Some(now_s));
        }
        (guidance::STOP, None)
    }

    fn arming(
        &mut self,
        state: &VehicleState,
        armed: bool,
        now_s: f64,
    ) -> (MotionDemand, Option<Discrete>) {
        if armed {
            let next = if state.height_m.is_some() {
                Phase::Climb
            } else {
                Phase::Enroute
            };
            self.enter(next, Some(now_s));
            return (guidance::STOP, None);
        }
        (guidance::STOP, self.request(Discrete::Arm, now_s))
    }

    fn climb(&mut self, state: &VehicleState, now_s: f64) -> (MotionDemand, Option<Discrete>) {
        let cruise = self.limits.cruise_height_m;
        let reached = state
            .height_m
            .is_none_or(|height| height >= cruise - CLIMB_CAPTURE_M);
        if reached {
            self.enter(Phase::Enroute, Some(now_s));
            return (guidance::hold_height(state, cruise), None);
        }
        let demand = MotionDemand {
            throttle: CLIMB_DEMAND,
            ..guidance::STOP
        };
        (demand, None)
    }

    fn enroute(&mut self, state: &VehicleState, now_s: f64) -> (MotionDemand, Option<Discrete>) {
        let cruise = guidance::height_demand(state, self.limits.cruise_height_m);
        let Some(destination) = self.destination.as_ref() else {
            return (self.on_station(state, cruise), None);
        };
        if self.enroute_since_s.is_none() {
            self.enroute_since_s = Some(now_s);
        }
        // A vehicle that crosses the radius at speed has not arrived. It
        // arrives when it is inside the radius and slow.
        let arrived = guidance::distance_m(state, destination.waypoint)
            <= self.limits.arrival_radius_m
            && guidance::ground_speed_mps(state) <= ARRIVAL_SPEED_MPS;
        if arrived {
            let next = match destination.on_arrival {
                Arrival::Hold => Phase::Holding,
                Arrival::Land => Phase::Descending,
            };
            self.enter(next, Some(now_s));
        }
        (self.on_station(state, cruise), None)
    }

    fn holding(&mut self, state: &VehicleState) -> (MotionDemand, Option<Discrete>) {
        self.served = true;
        let cruise = guidance::height_demand(state, self.limits.cruise_height_m);
        (self.on_station(state, cruise), None)
    }

    fn descending(&mut self, state: &VehicleState, now_s: f64) -> (MotionDemand, Option<Discrete>) {
        if self.touched_down(state, now_s) {
            // The flight is served at touchdown, not at disarm. A new
            // destination that arrives during the disarm must stay
            // unserved, so the vehicle flies it after it is disarmed.
            self.served = true;
            let next = if self.limits.disarm_offered {
                Phase::Disarming
            } else {
                Phase::Landed
            };
            self.enter(next, Some(now_s));
            return (guidance::STOP, None);
        }
        (self.on_station(state, DESCENT_DEMAND), None)
    }

    fn disarming(&mut self, armed: bool, now_s: f64) -> (MotionDemand, Option<Discrete>) {
        if !armed {
            self.enter(Phase::Landed, Some(now_s));
            return (guidance::STOP, None);
        }
        (guidance::STOP, self.request(Discrete::Disarm, now_s))
    }

    fn touched_down(&mut self, state: &VehicleState, now_s: f64) -> bool {
        let Some(height) = state.height_m else {
            return true;
        };
        if height <= GROUND_HEIGHT_M {
            return true;
        }
        let settled = height < SETTLED_BELOW_M && state.climb_mps.abs() < SETTLED_CLIMB_MPS;
        if !settled {
            self.settled_since_s = None;
            return false;
        }
        let since = *self.settled_since_s.get_or_insert(now_s);
        now_s - since >= SETTLED_FOR_S
    }

    /// Issues `request` now if none is out or the last one is overdue.
    fn request(&mut self, request: Discrete, now_s: f64) -> Option<Discrete> {
        let due = self
            .last_request_s
            .is_none_or(|last| now_s - last >= ACTION_RETRY_S);
        due.then(|| {
            self.last_request_s = Some(now_s);
            request
        })
    }
}

#[cfg(test)]
mod tests;

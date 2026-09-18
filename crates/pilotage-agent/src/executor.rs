//! The deterministic directive executor.
//!
//! A model supplies a directive and nothing else. Every decision that depends
//! on vehicle state is made here, in code that has no model in it: when to
//! arm, when the climb is complete, when the vehicle is at a fix, when it is
//! on the ground, when it is too far away. The executor has no I/O and no
//! clock of its own, so each decision can be tested.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::directive::{Arrival, Directive, HoldPoint, TurnDirection};
use crate::guidance::{Demand, Speeds};
use crate::model_port::Refusal;
use crate::scenario::{Fix, HOME, Procedure, Scenario};
use crate::state::VehicleState;

mod flight;

/// Interval between repeats of an arm or disarm request, in seconds.
const ACTION_RETRY_S: f64 = 2.0;

/// Where the flight is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// On the ground with nothing to fly.
    Idle,
    /// An arm request is out.
    Arming,
    /// Climbing to the commanded height.
    Climb,
    /// Flying to a fix.
    Enroute,
    /// Flying a heading.
    OnHeading,
    /// At a point, at the commanded height.
    Holding,
    /// At the range limit, at the commanded height. A heading took the
    /// vehicle there, and the executor stopped it.
    RangeHold,
    /// Descending to land.
    Descending,
    /// On the ground; a disarm request is out.
    Disarming,
    /// On the ground, and disarmed where the vehicle offers a disarm.
    Landed,
}

/// A discrete request for the reliable action channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discrete {
    /// Arm the vehicle.
    Arm,
    /// Disarm the vehicle.
    Disarm,
}

/// What the scenario and the vehicle advertisement fix for one flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlightLimits {
    /// Height above the launch point at the start, in metres.
    pub cruise_height_m: f64,
    /// Cruise speed at the start, in metres per second.
    pub cruise_speed_mps: f64,
    /// Horizontal distance that counts as arrival, in metres.
    pub arrival_radius_m: f64,
    /// The largest permitted distance from the launch point, in metres.
    pub max_range_m: f64,
    /// Advertised speed at full horizontal demand, in metres per second.
    pub max_linear_mps: f64,
    /// True when the motion scope advertises a disarm. A vehicle with no
    /// disarm to offer is landed when it is down.
    pub disarm_offered: bool,
}

/// The output of one executor step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    /// The motion demand for this frame.
    pub demand: Demand,
    /// A discrete request to send now, if one is due.
    pub action: Option<Discrete>,
    /// The phase after this step.
    pub phase: Phase,
}

/// What the vehicle is doing in the air.
#[derive(Debug, Clone, PartialEq)]
enum Task {
    /// Climb and then hold where the climb ends.
    Takeoff,
    /// Fly to a fix.
    GoTo { fix: Fix, arrival: Arrival },
    /// Fly fixes in sequence.
    Route {
        fixes: Vec<Fix>,
        next: usize,
        then: Arrival,
    },
    /// Fly a heading until the next directive or the range limit.
    FlyHeading {
        heading_rad: f64,
        turn: TurnDirection,
    },
    /// Stay at a point.
    HoldAt { fix: Fix },
    /// Descend at a point.
    LandAt { fix: Fix },
}

/// The directive state machine.
#[derive(Debug)]
pub struct Executor {
    limits: FlightLimits,
    fixes: BTreeMap<String, Fix>,
    procedures: BTreeMap<String, Procedure>,
    phase: Phase,
    task: Option<Task>,
    height_m: f64,
    speed_mps: f64,
    acknowledged_armed: bool,
    /// True when a climb did not leave the ground. The armed report is then
    /// not believed, and an arm request goes out until the host accepts one.
    force_arm: bool,
    climb_stalled_since_s: Option<f64>,
    last_request_s: Option<f64>,
    settled_since_s: Option<f64>,
    flying_since_s: Option<f64>,
}

impl Executor {
    /// An executor on the ground with nothing to fly.
    #[must_use]
    pub fn new(limits: FlightLimits, scenario: &Scenario) -> Self {
        Self {
            limits,
            fixes: scenario.fixes.clone(),
            procedures: scenario.procedures.clone(),
            phase: Phase::Idle,
            task: None,
            height_m: limits.cruise_height_m,
            speed_mps: limits.cruise_speed_mps,
            acknowledged_armed: false,
            force_arm: false,
            climb_stalled_since_s: None,
            last_request_s: None,
            settled_since_s: None,
            flying_since_s: None,
        }
    }

    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Seconds in the newest phase in the air, or `None` on the ground. An
    /// arrival, a hold and a new directive each start a new phase, so the
    /// count restarts at each of them.
    #[must_use]
    pub fn flying_seconds(&self, now_s: f64) -> Option<f64> {
        self.flying_since_s.map(|since| now_s - since)
    }

    /// Records the host's answer to a discrete request.
    pub fn on_action_result(&mut self, request: Discrete, accepted: bool) {
        if accepted {
            self.acknowledged_armed = request == Discrete::Arm;
            if request == Discrete::Arm {
                self.force_arm = false;
            }
        }
    }

    /// Takes a directive that passed the reply check. `state` gives the
    /// present position for the directives that need one.
    pub fn accept(
        &mut self,
        directive: &Directive,
        state: Option<&VehicleState>,
        now_s: f64,
    ) -> Result<(), Refusal> {
        // A directive that changes nothing is refused and not dropped: a
        // silent drop would go into the run record as a flown directive.
        let no_effect = Refusal::NoEffect {
            kind: directive.kind(),
            phase: self.phase,
        };
        let stays_down = matches!(
            directive,
            Directive::ReturnToBase {}
                | Directive::Land {}
                | Directive::Hold {
                    point: HoldPoint::PresentPosition,
                }
        );
        if self.phase == Phase::Arming && stays_down {
            self.stay_down(now_s);
            return Ok(());
        }
        let here = state.map(|state| Fix {
            north_m: state.north_m,
            east_m: state.east_m,
        });
        match directive {
            Directive::Unable { .. } => {}
            Directive::Speed { speed_mps } => self.speed_mps = *speed_mps,
            Directive::Altitude { height_m } => {
                self.height_m = *height_m;
                if let Some(task) = self.altitude_task(here, no_effect)? {
                    self.start(task, now_s);
                }
            }
            Directive::Land {} if self.phase == Phase::Climb => {
                let fix = self.present_fix(here, None, no_effect)?;
                self.task = Some(Task::LandAt { fix });
                self.enter(Phase::Descending, now_s);
            }
            _ => {
                let task = self.task_for(directive, here, state, no_effect)?;
                self.start(task, now_s);
            }
        }
        Ok(())
    }

    /// The task that a directive with a target starts.
    fn task_for(
        &mut self,
        directive: &Directive,
        here: Option<Fix>,
        state: Option<&VehicleState>,
        no_effect: Refusal,
    ) -> Result<Task, Refusal> {
        Ok(match directive {
            Directive::Takeoff {} if !self.on_ground() => return Err(no_effect),
            Directive::Takeoff {} => Task::Takeoff,
            Directive::DirectTo { fix, on_arrival } => Task::GoTo {
                fix: self.fix(fix)?,
                arrival: *on_arrival,
            },
            // A return from the ground is a takeoff that nobody asked for. A
            // vehicle that disarms is on the ground too.
            Directive::ReturnToBase {} if self.on_ground() || self.phase == Phase::Disarming => {
                return Err(no_effect);
            }
            Directive::ReturnToBase {} => Task::GoTo {
                fix: self.fix(HOME)?,
                arrival: Arrival::Land,
            },
            Directive::Hold {
                point: HoldPoint::Fix(fix),
            } => Task::GoTo {
                fix: self.fix(fix)?,
                arrival: Arrival::Hold,
            },
            Directive::Hold {
                point: HoldPoint::PresentPosition,
            } => Task::HoldAt {
                fix: self.present_fix(here, state, no_effect)?,
            },
            Directive::Heading { degrees, turn } => Task::FlyHeading {
                heading_rad: f64::from(*degrees).to_radians(),
                turn: *turn,
            },
            Directive::JoinProcedure { procedure } => self.route(procedure)?,
            Directive::Land {} => Task::LandAt {
                fix: self.present_fix(here, None, no_effect)?,
            },
            // A go-around ends the approach: the vehicle climbs to the cruise
            // height and holds where it is. It needs the air under it.
            Directive::GoAround {} if !self.airborne() => return Err(no_effect),
            Directive::GoAround {} => {
                self.height_m = self.limits.cruise_height_m;
                Task::HoldAt {
                    fix: here.ok_or(no_effect)?,
                }
            }
            Directive::Unable { .. } | Directive::Speed { .. } | Directive::Altitude { .. } => {
                return Err(no_effect);
            }
        })
    }

    /// What a new height starts: a hold that ends a descent where the
    /// vehicle is, a takeoff from the ground, or nothing in flight.
    fn altitude_task(
        &self,
        here: Option<Fix>,
        no_effect: Refusal,
    ) -> Result<Option<Task>, Refusal> {
        Ok(match self.phase {
            Phase::Descending => Some(Task::HoldAt {
                fix: here.ok_or(no_effect)?,
            }),
            Phase::Idle | Phase::Landed if self.task.is_none() => Some(Task::Takeoff),
            _ => None,
        })
    }

    /// The present position for a hold or a landing. A vehicle on the ground
    /// has nothing to hold or to land. During a climb a hold takes the
    /// present height, so the climb ends where the vehicle is.
    fn present_fix(
        &mut self,
        here: Option<Fix>,
        hold_state: Option<&VehicleState>,
        no_effect: Refusal,
    ) -> Result<Fix, Refusal> {
        if !(self.airborne() || self.phase == Phase::Climb) {
            return Err(no_effect);
        }
        let fix = here.ok_or(no_effect)?;
        if self.phase == Phase::Climb
            && let Some(height) = hold_state.and_then(|state| state.height_m)
        {
            self.height_m = height;
        }
        Ok(fix)
    }

    /// Ends an arming that the operator cancels. The arm request may be out,
    /// so the vehicle disarms when the scope offers a disarm.
    fn stay_down(&mut self, now_s: f64) {
        self.task = None;
        let next = if self.limits.disarm_offered {
            Phase::Disarming
        } else {
            Phase::Landed
        };
        self.enter(next, now_s);
    }

    /// Advances the machine by one frame.
    pub fn step(&mut self, state: &VehicleState, now_s: f64) -> Step {
        // The flight controller's word wins. The acknowledgement serves a
        // vehicle that reports no armed state.
        let armed = state.armed.unwrap_or(self.acknowledged_armed);
        let (demand, action) = match self.phase {
            Phase::Idle | Phase::Landed => {
                if self.task.is_some() {
                    self.enter(Phase::Arming, now_s);
                }
                (Demand::default(), None)
            }
            Phase::Arming => self.arming(state, armed, now_s),
            Phase::Disarming => self.disarming(armed, now_s),
            _ => (self.fly(state, now_s), None),
        };
        Step {
            demand,
            action,
            phase: self.phase,
        }
    }

    fn on_ground(&self) -> bool {
        matches!(self.phase, Phase::Idle | Phase::Landed)
    }

    fn airborne(&self) -> bool {
        matches!(
            self.phase,
            Phase::Enroute
                | Phase::OnHeading
                | Phase::Holding
                | Phase::RangeHold
                | Phase::Descending
        )
    }

    fn fix(&self, name: &str) -> Result<Fix, Refusal> {
        if name == HOME {
            return Ok(Fix {
                north_m: 0.0,
                east_m: 0.0,
            });
        }
        self.fixes
            .get(name)
            .copied()
            .ok_or_else(|| Refusal::UnknownFix(name.to_owned()))
    }

    fn route(&self, name: &str) -> Result<Task, Refusal> {
        let procedure = self
            .procedures
            .get(name)
            .ok_or_else(|| Refusal::UnknownProcedure(name.to_owned()))?;
        let fixes = procedure
            .fixes
            .iter()
            .map(|fix| self.fix(fix))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Task::Route {
            fixes,
            next: 0,
            then: procedure.then,
        })
    }

    /// Makes `task` the thing to fly. A vehicle in the air turns to it at
    /// once. A vehicle on the ground arms for it on the next step.
    fn start(&mut self, task: Task, now_s: f64) {
        let phase = phase_of(&task);
        self.task = Some(task);
        if self.airborne() {
            self.enter(phase, now_s);
        }
    }

    fn enter(&mut self, phase: Phase, now_s: f64) {
        self.phase = phase;
        self.last_request_s = None;
        self.settled_since_s = None;
        self.climb_stalled_since_s = None;
        // The flying time measures the newest phase in the air. A scripted
        // message that waits on it must not count the time of the phase
        // before: an arrival, a hold or a new directive each restart it.
        self.flying_since_s = matches!(
            phase,
            Phase::Enroute
                | Phase::OnHeading
                | Phase::Holding
                | Phase::RangeHold
                | Phase::Descending
        )
        .then_some(now_s);
    }

    fn speeds(&self) -> Speeds {
        Speeds {
            max_linear_mps: self.limits.max_linear_mps,
            cruise_mps: self.speed_mps.min(self.limits.max_linear_mps),
        }
    }

    fn arming(
        &mut self,
        state: &VehicleState,
        armed: bool,
        now_s: f64,
    ) -> (Demand, Option<Discrete>) {
        if !armed || self.force_arm {
            return (Demand::default(), self.request(Discrete::Arm, now_s));
        }
        if state.height_m.is_some() {
            self.enter(Phase::Climb, now_s);
        } else {
            self.begin_task(state, now_s);
        }
        (Demand::default(), None)
    }

    fn disarming(&mut self, armed: bool, now_s: f64) -> (Demand, Option<Discrete>) {
        if !armed {
            self.enter(Phase::Landed, now_s);
            return (Demand::default(), None);
        }
        (Demand::default(), self.request(Discrete::Disarm, now_s))
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

const fn phase_of(task: &Task) -> Phase {
    match task {
        Task::Takeoff | Task::HoldAt { .. } => Phase::Holding,
        Task::GoTo { .. } | Task::Route { .. } => Phase::Enroute,
        Task::FlyHeading { .. } => Phase::OnHeading,
        Task::LandAt { .. } => Phase::Descending,
    }
}

#[cfg(test)]
mod tests;

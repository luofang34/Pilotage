//! The phases in the air: climb, fly to a fix, fly a heading, hold, descend.

use super::{Executor, Phase, Task, phase_of};
use crate::directive::Arrival;
use crate::guidance::{self, Demand};
use crate::scenario::Fix;
use crate::state::VehicleState;

/// Climb demand from the ground to the commanded height. It is well above a
/// hover demand, so a flight controller that detects takeoff from the size of
/// the demand sees one.
const CLIMB_DEMAND: f32 = 0.8;
/// Descent demand for the landing.
const DESCENT_DEMAND: f32 = -0.4;
/// Height below the commanded height at which the climb is complete.
const CLIMB_CAPTURE_M: f64 = 0.5;
/// Ground speed above which a vehicle inside the arrival radius is still
/// passing through and is not at the fix, in metres per second.
const ARRIVAL_SPEED_MPS: f64 = 0.5;
/// Height at or below which the vehicle is on the ground.
const GROUND_HEIGHT_M: f64 = 0.3;
/// Height below which a vehicle that stopped descending is on the ground. It
/// covers an estimate that does not read zero at touchdown.
const SETTLED_BELOW_M: f64 = 1.5;
/// Vertical speed below which the vehicle is not descending.
const SETTLED_CLIMB_MPS: f64 = 0.15;
/// Time without descent that confirms touchdown, in seconds.
const SETTLED_FOR_S: f64 = 2.0;
/// Height below which a climb has not left the ground.
const STALLED_BELOW_M: f64 = 0.5;
/// Time on the ground under a climb demand that shows the vehicle is not
/// armed, in seconds.
const STALLED_FOR_S: f64 = 3.0;

/// What the en-route phase found at the present fix.
enum AtFix {
    /// Not at the fix.
    No,
    /// At a fix that is not the last one of a route.
    Passing,
    /// At the last fix and slow.
    Arrived(Fix, Arrival),
}

impl Executor {
    /// Starts the task phase at the end of the climb, or at once for a
    /// vehicle that does not climb.
    pub(super) fn begin_task(&mut self, state: &VehicleState, now_s: f64) {
        let here = Fix {
            north_m: state.north_m,
            east_m: state.east_m,
        };
        if matches!(self.task, None | Some(Task::Takeoff)) {
            self.task = Some(Task::HoldAt { fix: here });
        }
        let phase = self.task.as_ref().map_or(Phase::Holding, phase_of);
        self.enter(phase, now_s);
    }

    pub(super) fn fly(&mut self, state: &VehicleState, now_s: f64) -> Demand {
        match self.phase {
            Phase::Climb => self.climb(state, now_s),
            Phase::Enroute => self.enroute(state, now_s),
            Phase::OnHeading => self.on_heading(state, now_s),
            Phase::Descending => self.descending(state, now_s),
            _ => self.station(state),
        }
    }

    fn cruise_throttle(&self, state: &VehicleState) -> f32 {
        guidance::height_demand(state, self.height_m)
    }

    fn climb(&mut self, state: &VehicleState, now_s: f64) -> Demand {
        let reached = state
            .height_m
            .is_none_or(|height| height >= self.height_m - CLIMB_CAPTURE_M);
        if reached {
            self.begin_task(state, now_s);
            return Demand {
                throttle: self.cruise_throttle(state),
                ..Demand::default()
            };
        }
        // An armed report can be old: a flight controller that restarted is
        // disarmed, and the last report of the one before it said armed. A
        // vehicle that stays on the ground under a climb demand is not armed,
        // whatever the report says.
        let grounded = state
            .height_m
            .is_some_and(|height| height < STALLED_BELOW_M);
        if !grounded {
            self.climb_stalled_since_s = None;
        } else if now_s - *self.climb_stalled_since_s.get_or_insert(now_s) >= STALLED_FOR_S {
            self.force_arm = true;
            self.enter(Phase::Arming, now_s);
            return Demand::default();
        }
        Demand {
            throttle: CLIMB_DEMAND,
            ..Demand::default()
        }
    }

    fn enroute(&mut self, state: &VehicleState, now_s: f64) -> Demand {
        let Some(target) = self.target() else {
            return self.station(state);
        };
        match self.at_fix(state, target) {
            AtFix::No => {}
            AtFix::Passing => {
                if let Some(Task::Route { next, .. }) = self.task.as_mut() {
                    *next = next.wrapping_add(1);
                }
            }
            AtFix::Arrived(fix, Arrival::Hold) => {
                self.task = Some(Task::HoldAt { fix });
                self.enter(Phase::Holding, now_s);
            }
            AtFix::Arrived(fix, Arrival::Land) => {
                self.task = Some(Task::LandAt { fix });
                self.enter(Phase::Descending, now_s);
            }
        }
        let fix = self.target().unwrap_or(target);
        guidance::toward(state, fix, self.speeds(), self.cruise_throttle(state))
    }

    /// The fix that the vehicle flies to now.
    fn target(&self) -> Option<Fix> {
        match self.task.as_ref()? {
            Task::GoTo { fix, .. } | Task::HoldAt { fix } | Task::LandAt { fix } => Some(*fix),
            Task::Route { fixes, next, .. } => fixes.get(*next).or(fixes.last()).copied(),
            Task::Takeoff | Task::FlyHeading { .. } => None,
        }
    }

    fn at_fix(&self, state: &VehicleState, fix: Fix) -> AtFix {
        if guidance::distance_m(state, fix) > self.limits.arrival_radius_m {
            return AtFix::No;
        }
        let last = match self.task.as_ref() {
            Some(Task::GoTo { arrival, .. }) => Some(*arrival),
            Some(Task::Route { fixes, next, then }) => {
                (next.wrapping_add(1) >= fixes.len()).then_some(*then)
            }
            _ => None,
        };
        match last {
            None => AtFix::Passing,
            // A vehicle that crosses the radius at speed is not at the fix.
            // It is there when it is inside the radius and slow.
            Some(arrival) if guidance::ground_speed_mps(state) <= ARRIVAL_SPEED_MPS => {
                AtFix::Arrived(fix, arrival)
            }
            Some(_) => AtFix::No,
        }
    }

    fn on_heading(&mut self, state: &VehicleState, now_s: f64) -> Demand {
        let Some(Task::FlyHeading { heading_rad, turn }) = self.task.clone() else {
            return self.station(state);
        };
        // A heading has no end. The range limit is the end that protects the
        // vehicle when no directive follows.
        if state.north_m.hypot(state.east_m) > self.limits.max_range_m {
            self.task = Some(Task::HoldAt {
                fix: Fix {
                    north_m: state.north_m,
                    east_m: state.east_m,
                },
            });
            self.enter(Phase::RangeHold, now_s);
            return self.station(state);
        }
        let throttle = self.cruise_throttle(state);
        guidance::along_heading(state, heading_rad, turn, self.speeds(), throttle)
    }

    fn station(&self, state: &VehicleState) -> Demand {
        let throttle = self.cruise_throttle(state);
        self.target().map_or(
            Demand {
                throttle,
                ..Demand::default()
            },
            |fix| guidance::toward(state, fix, self.speeds(), throttle),
        )
    }

    fn descending(&mut self, state: &VehicleState, now_s: f64) -> Demand {
        if self.touched_down(state, now_s) {
            self.task = None;
            let next = if self.limits.disarm_offered {
                Phase::Disarming
            } else {
                Phase::Landed
            };
            self.enter(next, now_s);
            return Demand::default();
        }
        self.target().map_or(
            Demand {
                throttle: DESCENT_DEMAND,
                ..Demand::default()
            },
            |fix| guidance::toward(state, fix, self.speeds(), DESCENT_DEMAND),
        )
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
}

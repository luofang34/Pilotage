//! Which sticks to send: assisted flight (the outer loop on velocity
//! demands) or direct flight (rates or attitude passed through), and the
//! hand-over between them.
//!
//! The family is the one the adapter last received a frame for. A direct
//! demand that stops arriving falls back to assisted flight at zero
//! velocity, a hold, as a velocity demand that stops does; so does a
//! direct scope that leaves. Either hand-over starts from the throttle the
//! other family last sent, so a switch in flight neither drops nor jumps
//! the quad.

use std::time::{Duration, Instant};

use crate::direct::{DirectDemand, DirectLoop};
use crate::outer_loop::{Demand, Envelope, Estimate, OuterLoop};
use crate::rc::Sticks;

/// A demand older than this no longer flies the quad.
pub(crate) const DEMAND_TIMEOUT: Duration = Duration::from_millis(500);
/// Above this throttle a quad handed to assisted flight counts as
/// airborne; below it, assisted flight waits for a climb demand.
const AIRBORNE_THROTTLE: f64 = 0.15;

/// The control family a client flies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Family {
    /// Velocity demands through the host-side outer loop (ANGLE).
    #[default]
    Assisted,
    /// Body rates (ACRO) or attitude (ANGLE) with thrust, no host loop.
    Direct,
}

/// The demands the adapter last handed over.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Demands {
    /// The family of the latest frame.
    pub(crate) family: Family,
    /// The latest velocity demand and when it arrived.
    pub(crate) velocity: Demand,
    /// When the velocity demand arrived; `None` before the first.
    pub(crate) velocity_at: Option<Instant>,
    /// The latest direct demand.
    pub(crate) direct: Option<DirectDemand>,
    /// When the direct demand arrived.
    pub(crate) direct_at: Option<Instant>,
}

impl Demands {
    fn fresh(at: Option<Instant>, now: Instant) -> bool {
        at.is_some_and(|at| now.saturating_duration_since(at) <= DEMAND_TIMEOUT)
    }

    /// The direct demand that flies now, if the direct family is active
    /// and its demand fresh.
    fn active_direct(&self, now: Instant) -> Option<DirectDemand> {
        (self.family == Family::Direct && Self::fresh(self.direct_at, now))
            .then_some(self.direct)
            .flatten()
    }
}

/// The stick source and its hand-over state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pilot {
    outer: OuterLoop,
    direct: DirectLoop,
    in_use: Family,
    last: Sticks,
    heading: Option<f64>,
    armed: bool,
}

impl Pilot {
    pub(crate) fn new(envelope: Envelope) -> Self {
        Self {
            outer: OuterLoop::new(envelope),
            direct: DirectLoop::default(),
            in_use: Family::Assisted,
            last: Sticks::IDLE,
            heading: None,
            armed: false,
        }
    }

    /// Whether assisted flight counts the quad as airborne.
    pub(crate) fn is_flying(&self) -> bool {
        self.outer.is_flying()
    }

    /// The outer loop's learned hover throttle.
    pub(crate) fn hover_throttle(&self) -> f64 {
        self.outer.hover_throttle()
    }

    /// The sticks after a telemetry poll: `estimate` and `armed` are
    /// fresh, `arm` is the arm switch, `dt` the time since the last poll.
    pub(crate) fn on_poll(
        &mut self,
        demands: &Demands,
        estimate: &Estimate,
        armed: bool,
        arm: bool,
        now: Instant,
        dt: f64,
    ) -> Sticks {
        self.heading = Some(estimate.heading);
        self.armed = armed;
        if !armed {
            // Disarmed (or arming): sticks centred and throttle cut, which
            // Betaflight's arming check requires.
            self.outer.reset();
            self.in_use = Family::Assisted;
            return self.remember(Sticks {
                arm,
                ..Sticks::IDLE
            });
        }
        let sticks = match demands.active_direct(now) {
            Some(demand) => self.fly_direct(demand),
            None => {
                self.enter(Family::Assisted);
                let velocity = if Demands::fresh(demands.velocity_at, now) {
                    demands.velocity
                } else {
                    Demand::default()
                };
                self.outer.update(velocity, estimate, dt)
            }
        };
        self.remember(Sticks { arm, ..sticks })
    }

    /// The sticks to send at once for a direct demand that just arrived,
    /// or `None` when direct flight is not what flies now. Direct flight
    /// is the low-latency path: every frame leaves as soon as it arrives.
    pub(crate) fn on_demands(&mut self, demands: &Demands, now: Instant) -> Option<Sticks> {
        if !self.armed {
            return None;
        }
        let demand = demands.active_direct(now)?;
        let arm = self.last.arm;
        let sticks = self.fly_direct(demand);
        Some(self.remember(Sticks { arm, ..sticks }))
    }

    fn fly_direct(&mut self, demand: DirectDemand) -> Sticks {
        self.enter(Family::Direct);
        self.direct.sticks(demand, self.heading.unwrap_or_default())
    }

    fn enter(&mut self, family: Family) {
        if self.in_use == family {
            return;
        }
        let throttle = self.last.throttle;
        match family {
            // Airborne, direct flight starts at the learned hover rather than
            // at the assisted loop's last output: that output also carries
            // the climb-rate correction of the moment (pulling down after a
            // climb's overshoot, say), and a direct loop holding it would
            // keep sinking or climbing. On the ground it holds what was sent.
            Family::Direct => {
                let hover = self.outer.hover_throttle();
                let held = if self.outer.is_flying() {
                    hover
                } else {
                    throttle
                };
                self.direct.begin(held, hover);
            }
            Family::Assisted => self.outer.resume(throttle, throttle > AIRBORNE_THROTTLE),
        }
        tracing::info!(?family, throttle, "Betaflight control family changed");
        self.in_use = family;
    }

    fn remember(&mut self, sticks: Sticks) -> Sticks {
        self.last = sticks;
        sticks
    }
}

#[cfg(test)]
mod tests;

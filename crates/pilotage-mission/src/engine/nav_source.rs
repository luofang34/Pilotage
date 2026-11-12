//! The navigation source machine: whether the configured position source
//! gives a solution to steer on, and on what terms.
//!
//! Two pure functions. [`classify`] turns what the source holds, the
//! limits the mission flies under, and whether the vehicle is in flight
//! into one [`NavEvent`]. [`next`] turns a state and that event into the
//! next state and what there is to announce. Neither reads the mission's
//! phase: flight is handed in as a fact, so no behaviour here is implied
//! by another machine's state.
//!
//! Each tick steps the machine once ([`MissionEngine::judge_source`]),
//! and its state alone decides whether the held solution is steered on,
//! whether by dead reckoning, and why not when it is not.

use navigate_contract::{DurationNanos, MonotonicNanos, NavigationSolution, SolutionQuality};

use crate::config::{BeyondLimits, OwnshipSource};

use super::estimate::EstimateBudget;
use super::{MissionEngine, MissionEvent, StartWithheldReason};

#[cfg(test)]
mod tests;

/// Why the source gives nothing to steer on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Loss {
    /// What it holds is older than its age limit.
    Stale,
    /// The navigator grades its solution unusable.
    Unusable,
    /// The solution's horizontal uncertainty is above the limit, or is
    /// not a number.
    TooUncertain,
}

/// The state of the position source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) enum NavState {
    /// The source has given nothing yet.
    #[default]
    NotStarted,
    /// A solution within the limits is steered on.
    Fit,
    /// A solution beyond the limits is steered on all the same.
    DeadReckoning,
    /// Nothing is steered on.
    Lost(Loss),
}

impl NavState {
    /// Whether a solution is steered on.
    pub(super) const fn steers(self) -> bool {
        matches!(self, Self::Fit | Self::DeadReckoning)
    }
}

/// What one tick found of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum NavEvent {
    /// The source holds nothing.
    NothingHeld,
    /// What it holds is older than its age limit.
    Aged,
    /// It holds a fresh solution within the limits.
    WithinLimits,
    /// It holds a fresh solution beyond the limits that the policy flies.
    DeadReckoned,
    /// It holds a fresh solution graded unusable that the policy does not
    /// fly.
    GradedUnusable,
    /// It holds a fresh solution too uncertain to fly.
    TooUncertain,
}

/// What a transition gives to announce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum NavEffect {
    /// Nothing changed that an operator needs to hear.
    None,
    /// Dead reckoning began.
    DeadReckoningBegan,
    /// A solution within the limits is steered on again after dead
    /// reckoning or a loss.
    Recovered,
    /// The source was lost, or the reason it is lost changed.
    Lost(Loss),
}

/// What the source holds at one tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Held {
    /// Its age is within the source's limit.
    pub(super) fresh: bool,
    /// The navigator grades it unusable.
    pub(super) graded_unusable: bool,
    /// Its 1-sigma horizontal uncertainty, metres.
    pub(super) horizontal_1sigma_m: f64,
}

/// The limits a held solution is judged against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Limits {
    /// The 1-sigma horizontal uncertainty above which a solution is
    /// beyond its limits, metres.
    pub(super) max_horizontal_1sigma_m: f64,
    /// What a fresh solution beyond its limits does.
    pub(super) beyond: BeyondLimits,
}

/// The event of one tick. A solution beyond its limits is flown by dead
/// reckoning only when the policy says so, the vehicle is in flight, and
/// the uncertainty is a number under the policy's ceiling: before flight
/// the start waits for a solution within the limits, and an unknown
/// uncertainty is never flown.
pub(super) fn classify(held: Option<Held>, limits: Limits, in_flight: bool) -> NavEvent {
    let Some(held) = held else {
        return NavEvent::NothingHeld;
    };
    if !held.fresh {
        return NavEvent::Aged;
    }
    let sigma = held.horizontal_1sigma_m;
    if !held.graded_unusable && sigma <= limits.max_horizontal_1sigma_m {
        return NavEvent::WithinLimits;
    }
    if let BeyondLimits::DeadReckon { ceiling_1sigma_m } = limits.beyond
        && in_flight
        && sigma.is_finite()
        && ceiling_1sigma_m.is_none_or(|ceiling| sigma <= ceiling)
    {
        return NavEvent::DeadReckoned;
    }
    if held.graded_unusable {
        NavEvent::GradedUnusable
    } else {
        NavEvent::TooUncertain
    }
}

/// The state after `event` in `state`, and what to announce. The next
/// state follows from the event alone, except that a source that has
/// never given anything is not lost; the announcement follows from the
/// change.
pub(super) const fn next(state: NavState, event: NavEvent) -> (NavState, NavEffect) {
    let to = match event {
        NavEvent::NothingHeld => match state {
            NavState::NotStarted => NavState::NotStarted,
            _ => NavState::Lost(Loss::Stale),
        },
        NavEvent::Aged => NavState::Lost(Loss::Stale),
        NavEvent::WithinLimits => NavState::Fit,
        NavEvent::DeadReckoned => NavState::DeadReckoning,
        NavEvent::GradedUnusable => NavState::Lost(Loss::Unusable),
        NavEvent::TooUncertain => NavState::Lost(Loss::TooUncertain),
    };
    let effect = match (state, to) {
        (NavState::DeadReckoning | NavState::Lost(_), NavState::Fit) => NavEffect::Recovered,
        (NavState::DeadReckoning, NavState::DeadReckoning) => NavEffect::None,
        (_, NavState::DeadReckoning) => NavEffect::DeadReckoningBegan,
        (NavState::Lost(from), NavState::Lost(loss)) if from as u8 == loss as u8 => NavEffect::None,
        (_, NavState::Lost(loss)) => NavEffect::Lost(loss),
        _ => NavEffect::None,
    };
    (to, effect)
}

/// The machine's state, its dead-reckoning and hold annunciations, and why
/// its latest tick gave no solution.
#[derive(Debug, Default)]
pub(super) struct NavMode {
    /// The machine's state after the latest tick that read the source.
    pub(super) state: NavState,
    /// Why the latest tick published no solution; `None` while one is
    /// published or before any tick.
    pub(super) no_solution: Option<StartWithheldReason>,
    /// The horizontal uncertainty of the dead reckoning last reported,
    /// metres; `None` while none is reported.
    pub(super) reported_dead_reckoning_m: Option<f64>,
    /// Why the engine holds heading or position, while it does.
    pub(super) holding: Option<StartWithheldReason>,
}

/// What the configured source holds at one tick.
struct Reading {
    /// The event the guards chose.
    event: NavEvent,
    /// The solution steered on when the machine says one is.
    held: Option<NavigationSolution>,
    /// The horizontal limit the held solution was judged against, metres.
    limit_m: f64,
}

impl MissionEngine {
    /// The solution steered on at `now`, or `None`: reads the configured
    /// source once, steps the machine, takes the held solution only in a
    /// state that steers, and emits dead-reckoning / recovery annunciations
    /// when `events` is supplied.
    pub(super) fn judge_source(
        &mut self,
        now: MonotonicNanos,
        events: Option<&mut Vec<MissionEvent>>,
    ) -> Option<NavigationSolution> {
        let reading = self.read_source(now);
        let (state, _) = next(self.modes.nav.state, reading.event);
        self.modes.nav.state = state;
        let solution = reading.held.filter(|_| state.steers());
        if state.steers() {
            self.modes.nav.holding = None;
        }
        self.onboard.dead_reckoned = solution.filter(|_| state == NavState::DeadReckoning);
        self.modes.nav.no_solution = match state {
            NavState::Fit | NavState::DeadReckoning => None,
            NavState::NotStarted => Some(StartWithheldReason::NavigationNotStarted),
            NavState::Lost(Loss::Stale) => Some(StartWithheldReason::NavigationStale),
            NavState::Lost(Loss::Unusable) => Some(StartWithheldReason::NavigationUnusable),
            NavState::Lost(Loss::TooUncertain) => {
                Some(StartWithheldReason::NavigationTooUncertain {
                    horizontal_1sigma_m: reading
                        .held
                        .map_or(f64::NAN, |held| held.integrity.horizontal_1sigma_m),
                    limit_m: reading.limit_m,
                })
            }
        };
        if let Some(events) = events {
            self.announce_dead_reckoning(solution.as_ref(), events);
        }
        solution
    }

    /// Emits [`MissionEvent::DeadReckoning`] on entry and each doubling of
    /// horizontal uncertainty, and [`MissionEvent::NavigationRecovered`]
    /// when a solution within limits returns directly from dead reckoning.
    fn announce_dead_reckoning(
        &mut self,
        solution: Option<&NavigationSolution>,
        events: &mut Vec<MissionEvent>,
    ) {
        let reported = self.modes.nav.reported_dead_reckoning_m;
        match (self.dead_reckoning(), reported) {
            (Some(now), last) => {
                let sigma = now.horizontal_1sigma_m;
                if last.is_none_or(|last| sigma >= 2.0 * last) {
                    self.modes.nav.reported_dead_reckoning_m = Some(sigma);
                    events.push(MissionEvent::DeadReckoning {
                        horizontal_1sigma_m: sigma,
                        vertical_1sigma_m: now.vertical_1sigma_m,
                    });
                }
            }
            (None, Some(_)) => {
                self.modes.nav.reported_dead_reckoning_m = None;
                if let Some(sol) = solution {
                    events.push(MissionEvent::NavigationRecovered {
                        horizontal_1sigma_m: sol.integrity.horizontal_1sigma_m,
                    });
                }
            }
            (None, None) => {}
        }
    }

    /// Announces the hold of a vehicle with no solution fit to fly, once
    /// for each reason in a row.
    pub(super) fn announce_hold(&mut self, events: &mut Vec<MissionEvent>) {
        let reason = self.solution_absence();
        let changed =
            self.modes.nav.holding.is_none_or(|held| {
                core::mem::discriminant(&held) != core::mem::discriminant(&reason)
            });
        self.modes.nav.holding = Some(reason);
        if changed {
            events.push(MissionEvent::NavigationHold { reason });
        }
    }

    /// Why the latest tick gave no solution to steer on.
    pub(super) fn solution_absence(&self) -> StartWithheldReason {
        self.modes
            .nav
            .no_solution
            .unwrap_or(StartWithheldReason::NavigationNotStarted)
    }

    /// What the configured source holds at `now`, with the refusal
    /// counted under the source's own counter.
    fn read_source(&mut self, now: MonotonicNanos) -> Reading {
        // Flight is the lifecycle's fact, handed to the machine as a guard.
        let in_flight = self.lifecycle().in_flight();
        match self.config.ownship_source {
            OwnshipSource::OnboardNavigation {
                max_age,
                max_horizontal_1sigma_m,
                beyond_limits,
            } => {
                let limits = Limits {
                    max_horizontal_1sigma_m,
                    beyond: beyond_limits,
                };
                self.read_onboard(now, max_age, limits, in_flight)
            }
            OwnshipSource::VehicleEstimate {
                sigma_m,
                velocity_sigma_mps,
                max_age,
            } => {
                let facts = self.estimate.acquired_at().map(|acquired_at| Held {
                    fresh: is_fresh(now, acquired_at, max_age),
                    graded_unusable: false,
                    horizontal_1sigma_m: sigma_m,
                });
                let event = classify(facts, UNLIMITED, in_flight);
                if event == NavEvent::Aged {
                    self.counters.estimate_stale = self.counters.estimate_stale.wrapping_add(1);
                }
                let budget = EstimateBudget {
                    sigma_m,
                    velocity_sigma_mps,
                    max_age,
                };
                Reading {
                    event,
                    held: self
                        .estimate
                        .solution(now, &self.plane, self.config.clock, budget),
                    limit_m: UNLIMITED.max_horizontal_1sigma_m,
                }
            }
            // The fusion filter judges its own solution: it publishes one
            // or it does not, and a filter that was given samples and
            // publishes nothing has aged out.
            OwnshipSource::SimulationTruth => {
                let held = self.filter.tick(now);
                let event = match held {
                    Some(_) => NavEvent::WithinLimits,
                    None if self.truth_sampled => NavEvent::Aged,
                    None => NavEvent::NothingHeld,
                };
                Reading {
                    event,
                    held,
                    limit_m: UNLIMITED.max_horizontal_1sigma_m,
                }
            }
        }
    }
}

impl MissionEngine {
    /// What the onboard navigator's latest solution is at `now` against
    /// `limits`, with the refusal counted.
    fn read_onboard(
        &mut self,
        now: MonotonicNanos,
        max_age: DurationNanos,
        limits: Limits,
        in_flight: bool,
    ) -> Reading {
        let held = self.onboard.held().copied();
        let facts = held.map(|held| Held {
            fresh: is_fresh(now, held.stamp.solved_at, max_age),
            graded_unusable: held.integrity.quality == SolutionQuality::Unusable,
            horizontal_1sigma_m: held.integrity.horizontal_1sigma_m,
        });
        let event = classify(facts, limits, in_flight);
        let counter = match event {
            NavEvent::Aged => Some(&mut self.counters.onboard_stale),
            NavEvent::DeadReckoned => Some(&mut self.counters.onboard_dead_reckoned),
            NavEvent::GradedUnusable | NavEvent::TooUncertain => {
                Some(&mut self.counters.onboard_unfit)
            }
            NavEvent::NothingHeld | NavEvent::WithinLimits => None,
        };
        if let Some(counter) = counter {
            *counter = counter.wrapping_add(1);
        }
        Reading {
            event,
            held,
            limit_m: limits.max_horizontal_1sigma_m,
        }
    }
}

/// Whether a value acquired at `at` is no older than `max_age` at `now`.
/// A value stamped after `now` has no age and is not fresh.
fn is_fresh(now: MonotonicNanos, at: MonotonicNanos, max_age: DurationNanos) -> bool {
    now.elapsed_since(at)
        .is_some_and(|age| age.as_nanos() <= max_age.as_nanos())
}

/// The limits of a source that states no uncertainty of its own to judge.
const UNLIMITED: Limits = Limits {
    max_horizontal_1sigma_m: f64::INFINITY,
    beyond: BeyondLimits::Hold,
};

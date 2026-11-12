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
//! The engine runs the machine beside the source's own judgment each tick
//! and counts every tick on which the two disagree
//! ([`super::MissionCounters::nav_source_mismatch`]).

use navigate_contract::{MonotonicNanos, SolutionQuality};

use crate::config::{BeyondLimits, OwnshipSource};

use super::{MissionEngine, MissionState, StartWithheldReason};

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

/// The machine's state beside what the display reads of a tick with no
/// solution.
#[derive(Debug, Default)]
pub(super) struct NavMode {
    /// The machine's state after the latest tick that read the source.
    pub(super) state: NavState,
    /// Why the latest tick published no solution; `None` while one is
    /// published or before any tick.
    pub(super) no_solution: Option<StartWithheldReason>,
}

impl MissionEngine {
    /// Steps the machine for a tick at `now` whose source judgment is
    /// already made, and counts a disagreement with that judgment:
    /// whether a solution is steered on, and whether by dead reckoning.
    pub(super) fn step_nav_source(&mut self, now: MonotonicNanos) {
        let event = self.nav_event(now);
        let (state, _) = next(self.nav.state, event);
        self.nav.state = state;
        let steers = self.last_solution.is_some();
        let dead_reckons = self.onboard.dead_reckoned.is_some();
        if state.steers() != steers || (state == NavState::DeadReckoning) != dead_reckons {
            self.counters.nav_source_mismatch = self.counters.nav_source_mismatch.wrapping_add(1);
        }
    }

    /// The event of a tick at `now` for the configured source.
    fn nav_event(&self, now: MonotonicNanos) -> NavEvent {
        // Flight is the mission's fact, handed to the machine as a guard.
        let in_flight = matches!(
            self.state(),
            MissionState::Climb | MissionState::Enroute | MissionState::Complete
        );
        match self.config.ownship_source {
            OwnshipSource::OnboardNavigation {
                max_age,
                max_horizontal_1sigma_m,
                beyond_limits,
            } => {
                let held = self.onboard.held().map(|held| Held {
                    fresh: now
                        .elapsed_since(held.stamp.solved_at)
                        .is_some_and(|age| age.as_nanos() <= max_age.as_nanos()),
                    graded_unusable: held.integrity.quality == SolutionQuality::Unusable,
                    horizontal_1sigma_m: held.integrity.horizontal_1sigma_m,
                });
                let limits = Limits {
                    max_horizontal_1sigma_m,
                    beyond: beyond_limits,
                };
                classify(held, limits, in_flight)
            }
            OwnshipSource::VehicleEstimate {
                sigma_m, max_age, ..
            } => {
                let held = self.estimate.acquired_at().map(|acquired_at| Held {
                    fresh: now
                        .elapsed_since(acquired_at)
                        .is_some_and(|age| age.as_nanos() <= max_age.as_nanos()),
                    graded_unusable: false,
                    horizontal_1sigma_m: sigma_m,
                });
                classify(held, UNLIMITED, in_flight)
            }
            // The fusion filter judges its own solution: it publishes one
            // or it does not, and a filter that was given samples and
            // publishes nothing has aged out.
            OwnshipSource::SimulationTruth => match self.last_solution {
                Some(_) => NavEvent::WithinLimits,
                None if self.truth_sampled => NavEvent::Aged,
                None => NavEvent::NothingHeld,
            },
        }
    }
}

/// The limits of a source that states no uncertainty of its own to judge.
const UNLIMITED: Limits = Limits {
    max_horizontal_1sigma_m: f64::INFINITY,
    beyond: BeyondLimits::Hold,
};

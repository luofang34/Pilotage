//! The onboard navigator's solution as the engine's ownship.
//!
//! The navigator fuses its own sources into a navigation solution
//! (ADR-0024). The host hands each published solution to the engine. The
//! engine runs no fusion on it and does not change its covariance,
//! integrity, or composition: it flies the latest solution while that
//! solution is fresh and fit, and publishes nothing otherwise.
//!
//! Age is the one judgment the engine cannot take from the navigator. A
//! solution stamped on the mission clock keeps its stamp. A solution
//! stamped on another clock is not comparable with `now`, so its age
//! counts from the moment the host offered it. A repeat of the held
//! solution is not a new solution: the navigator's sequence decides what
//! is new, so a navigator that stops solving goes stale even when its
//! host keeps offering the last solution.

use navigate_contract::{
    AttitudeQuaternion, DurationNanos, MonotonicNanos, NavigationSolution, SolutionQuality,
};

use crate::config::{BeyondLimits, OwnshipSource};

use super::{MissionEngine, StartWithheldReason};

/// How far a quaternion norm may be from one and still give a heading.
const UNIT_NORM_TOLERANCE: f64 = 1e-3;

/// The bounds a held solution must meet to be flown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct OnboardLimits {
    /// Age past which the held solution yields no solution.
    pub(super) max_age: DurationNanos,
    /// 1-sigma horizontal uncertainty above which the held solution is
    /// beyond its limits, meters.
    pub(super) max_horizontal_1sigma_m: f64,
    /// What a fresh solution beyond its limits does.
    pub(super) beyond: BeyondLimits,
}

/// The latest solution the navigator published, on the mission clock.
#[derive(Debug, Default)]
pub(super) struct OnboardNavigation {
    latest: Option<NavigationSolution>,
    /// The solution flown by dead reckoning on the latest tick, if one
    /// was: fresh, beyond its limits, and flown all the same.
    pub(super) dead_reckoned: Option<NavigationSolution>,
    /// The horizontal uncertainty of the dead reckoning last reported,
    /// meters; `None` while none is reported.
    pub(super) reported_dead_reckoning_m: Option<f64>,
}

impl OnboardNavigation {
    /// The latest solution kept, whatever its age or fitness.
    pub(super) const fn held(&self) -> Option<&NavigationSolution> {
        self.latest.as_ref()
    }
}

impl MissionEngine {
    /// Offers one solution of the onboard navigator. `now` is the time of
    /// the offer on the engine's clock domain.
    ///
    /// The engine keeps the solution when it is on the mission's
    /// ellipsoid, its position is a plausible geodetic position, every
    /// velocity component is finite, its stamp is not later than `now` on
    /// the mission clock, and it is newer than the held solution of the
    /// same navigator epoch. Every other solution is a counted refusal.
    ///
    /// Under [`OwnshipSource::OnboardNavigation`] the kept solution is the
    /// ownship, and its attitude gives the heading that rotates commands
    /// into the body frame: a solution without an attitude clears the
    /// heading. Under another source the engine keeps the solution and
    /// does not fly it.
    pub fn on_onboard_solution(&mut self, solution: &NavigationSolution, now: MonotonicNanos) {
        let Some(held) = self.admit_onboard(solution, now) else {
            return;
        };
        if matches!(
            self.config.ownship_source,
            OwnshipSource::OnboardNavigation { .. }
        ) {
            self.last_yaw_rad = held.attitude.as_ref().and_then(heading_rad);
        }
        self.onboard.latest = Some(held);
    }

    /// The solution to keep for an offered one, on the mission clock, or
    /// `None` with the refusal counted.
    fn admit_onboard(
        &mut self,
        solution: &NavigationSolution,
        now: MonotonicNanos,
    ) -> Option<NavigationSolution> {
        let repeated = self.onboard.latest.as_ref().is_some_and(|held| {
            held.stamp.epoch == solution.stamp.epoch
                && !held.stamp.sequence.admits(solution.stamp.sequence)
        });
        if repeated {
            self.counters.onboard_repeated = self.counters.onboard_repeated.wrapping_add(1);
            return None;
        }
        let mut held = *solution;
        if held.stamp.clock != self.config.clock {
            held.stamp.solved_at = now;
            held.stamp.clock = self.config.clock;
        }
        let usable = held.ellipsoid == crate::body::ELLIPSOID
            && solution.position.is_plausible()
            && velocity_is_finite(solution)
            && now.elapsed_since(held.stamp.solved_at).is_some();
        if !usable {
            self.counters.onboard_refused = self.counters.onboard_refused.wrapping_add(1);
            return None;
        }
        Some(held)
    }

    /// The held solution for `now`, or `None` when none is held, the held
    /// one is older than `limits` allow, or it is beyond its limits (graded
    /// unusable, or with a horizontal uncertainty above the limit) and not
    /// flown by dead reckoning.
    pub(super) fn onboard_solution(
        &mut self,
        now: MonotonicNanos,
        limits: OnboardLimits,
    ) -> Option<NavigationSolution> {
        self.onboard.dead_reckoned = None;
        let held = self.onboard.latest?;
        let fresh = now
            .elapsed_since(held.stamp.solved_at)
            .is_some_and(|age| age.as_nanos() <= limits.max_age.as_nanos());
        if !fresh {
            self.counters.onboard_stale = self.counters.onboard_stale.wrapping_add(1);
            return None;
        }
        if within_limits(&held, limits) {
            return Some(held);
        }
        if self.dead_reckons(&held, limits) {
            self.counters.onboard_dead_reckoned =
                self.counters.onboard_dead_reckoned.wrapping_add(1);
            self.onboard.dead_reckoned = Some(held);
            return Some(held);
        }
        self.counters.onboard_unfit = self.counters.onboard_unfit.wrapping_add(1);
        None
    }

    /// Whether `held`, beyond its limits, is flown by dead reckoning: the
    /// policy says so, the vehicle is in flight, and the uncertainty is a
    /// number under the ceiling. Before flight the start waits for a
    /// solution within the limits; a solution of unknown uncertainty is
    /// never flown.
    fn dead_reckons(&self, held: &NavigationSolution, limits: OnboardLimits) -> bool {
        let BeyondLimits::DeadReckon { ceiling_1sigma_m } = limits.beyond else {
            return false;
        };
        let sigma = held.integrity.horizontal_1sigma_m;
        let in_flight = matches!(
            self.state(),
            super::MissionState::Climb
                | super::MissionState::Enroute
                | super::MissionState::Complete
        );
        in_flight && sigma.is_finite() && ceiling_1sigma_m.is_none_or(|ceiling| sigma <= ceiling)
    }
}

impl MissionEngine {
    /// Why the onboard navigator gives no solution to fly at `now`: none
    /// published yet, the held one too old, graded unusable, or with a
    /// horizontal uncertainty above the limit. `None` when the held
    /// solution is flown, within its limits or by dead reckoning. Reads the same judgments as
    /// [`Self::onboard_solution`] and counts nothing.
    pub(super) fn onboard_absence(
        &self,
        now: MonotonicNanos,
        limits: OnboardLimits,
    ) -> Option<StartWithheldReason> {
        let Some(held) = self.onboard.latest else {
            return Some(StartWithheldReason::NavigationNotStarted);
        };
        let fresh = now
            .elapsed_since(held.stamp.solved_at)
            .is_some_and(|age| age.as_nanos() <= limits.max_age.as_nanos());
        if !fresh {
            return Some(StartWithheldReason::NavigationStale);
        }
        if within_limits(&held, limits) || self.dead_reckons(&held, limits) {
            return None;
        }
        if held.integrity.quality == SolutionQuality::Unusable {
            return Some(StartWithheldReason::NavigationUnusable);
        }
        let sigma = held.integrity.horizontal_1sigma_m;
        (sigma > limits.max_horizontal_1sigma_m).then_some(
            StartWithheldReason::NavigationTooUncertain {
                horizontal_1sigma_m: sigma,
                limit_m: limits.max_horizontal_1sigma_m,
            },
        )
    }
}

/// Whether `held` is within `limits`: not graded unusable, and with a
/// horizontal uncertainty no larger than the limit.
fn within_limits(held: &NavigationSolution, limits: OnboardLimits) -> bool {
    held.integrity.quality != SolutionQuality::Unusable
        && held.integrity.horizontal_1sigma_m <= limits.max_horizontal_1sigma_m
}

/// Whether every velocity component is a finite number.
fn velocity_is_finite(solution: &NavigationSolution) -> bool {
    [
        solution.velocity.north_mps,
        solution.velocity.east_mps,
        solution.velocity.down_mps,
    ]
    .iter()
    .all(|value| value.is_finite())
}

/// Heading of a body-to-NED quaternion, radians, zero north, positive
/// toward east. `None` for a quaternion that is not finite or not of
/// unit norm: it states no direction.
fn heading_rad(attitude: &AttitudeQuaternion) -> Option<f64> {
    let (w, x, y, z) = (attitude.w, attitude.x, attitude.y, attitude.z);
    let norm = (w * w + x * x + y * y + z * z).sqrt();
    if !norm.is_finite() || (norm - 1.0).abs() > UNIT_NORM_TOLERANCE {
        return None;
    }
    Some((2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z)))
}

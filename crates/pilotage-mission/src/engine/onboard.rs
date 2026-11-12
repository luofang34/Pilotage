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

use navigate_contract::{AttitudeQuaternion, MonotonicNanos, NavigationSolution};

use crate::config::OwnshipSource;

use super::{MissionEngine, OnboardSolution, SourceIntegrity};

/// How far a quaternion norm may be from one and still give a heading.
const UNIT_NORM_TOLERANCE: f64 = 1e-3;

/// The latest solution the navigator published, on the mission clock.
#[derive(Debug, Default)]
pub(super) struct OnboardNavigation {
    latest: Option<NavigationSolution>,
    /// What the navigator said of a position sensor with the latest
    /// solution kept.
    source_integrity: Option<SourceIntegrity>,
    /// The solution flown by dead reckoning on the latest tick, if one
    /// was: fresh, beyond its limits, and flown all the same.
    pub(super) dead_reckoned: Option<NavigationSolution>,
}

impl OnboardNavigation {
    /// What the navigator said of a position sensor with the latest
    /// solution kept.
    pub(super) const fn source_integrity(&self) -> Option<SourceIntegrity> {
        self.source_integrity
    }

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
    ///
    /// What the navigator says of a position sensor is kept with the
    /// solution it came with, and dropped with a refused one.
    pub fn on_onboard_solution(
        &mut self,
        offered: impl Into<OnboardSolution>,
        now: MonotonicNanos,
    ) {
        let offered = offered.into();
        let Some(held) = self.admit_onboard(&offered.solution, now) else {
            return;
        };
        self.onboard.source_integrity = offered.source_integrity;
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

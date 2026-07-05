//! Staleness policy: converts a control-frame age into an accept/reject signal.
//!
//! ADR-0009 requires the session host to reject frames older than a configured
//! maximum control age. `StalenessPolicy` centralizes that threshold so hosts
//! do not scatter raw `Duration` comparisons through the validation path.

use core::time::Duration;

/// Configured maximum age for control frames before they are considered stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StalenessPolicy {
    /// The oldest age, inclusive, at which a control frame is still accepted.
    ///
    /// An age exactly equal to this bound is classified `Stale`: the bound is
    /// the first rejected age, not the last accepted one. This closed-on-the-
    /// stale-side choice means a configured "reject at 50ms" reads literally.
    max_control_age: Duration,
}

/// Outcome of comparing a frame's estimated age against a `StalenessPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// The frame's age is strictly less than the policy's maximum.
    Fresh,
    /// The frame's age is at or beyond the policy's maximum and MUST be
    /// rejected per ADR-0009.
    Stale {
        /// The age that triggered rejection.
        age: Duration,
    },
}

impl StalenessPolicy {
    /// Constructs a policy from a maximum control age.
    #[must_use]
    pub const fn new(max_control_age: Duration) -> Self {
        Self { max_control_age }
    }

    /// Returns the configured maximum control age.
    #[must_use]
    pub const fn max_control_age(&self) -> Duration {
        self.max_control_age
    }

    /// Classifies `age` as `Fresh` or `Stale` against this policy.
    ///
    /// `age == max_control_age` is `Stale`: the boundary belongs to the
    /// rejected side, matching ADR-0009's "frames older than the configured
    /// maximum" wording read as "at or past the maximum are rejected".
    #[must_use]
    pub fn check(&self, age: Duration) -> Freshness {
        if age >= self.max_control_age {
            Freshness::Stale { age }
        } else {
            Freshness::Fresh
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Duration, Freshness, StalenessPolicy};

    #[test]
    fn age_below_max_is_fresh() {
        let policy = StalenessPolicy::new(Duration::from_millis(50));
        assert_eq!(policy.check(Duration::from_millis(49)), Freshness::Fresh);
    }

    #[test]
    fn age_equal_to_max_is_stale() {
        let policy = StalenessPolicy::new(Duration::from_millis(50));
        assert_eq!(
            policy.check(Duration::from_millis(50)),
            Freshness::Stale {
                age: Duration::from_millis(50)
            }
        );
    }

    #[test]
    fn age_above_max_is_stale() {
        let policy = StalenessPolicy::new(Duration::from_millis(50));
        assert_eq!(
            policy.check(Duration::from_millis(51)),
            Freshness::Stale {
                age: Duration::from_millis(51)
            }
        );
    }
}

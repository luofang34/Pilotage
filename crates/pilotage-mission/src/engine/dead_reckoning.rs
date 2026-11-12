//! Flight on an onboard solution beyond its limits: the plan goes on by
//! dead reckoning under [`crate::BeyondLimits::DeadReckon`], and the
//! engine exposes the uncertainty it flies on for display context.

use super::{MissionEngine, NavQuality};

/// The onboard solution flown by dead reckoning on the latest tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeadReckoning {
    /// Its 1-sigma horizontal uncertainty, meters.
    pub horizontal_1sigma_m: f64,
    /// Its 1-sigma vertical uncertainty, meters.
    pub vertical_1sigma_m: f64,
    /// Its integrity grade.
    pub quality: NavQuality,
}

impl MissionEngine {
    /// The dead reckoning the latest [`MissionEngine::tick`] flew, or
    /// `None` when it flew a solution within its limits or none.
    #[must_use]
    pub fn dead_reckoning(&self) -> Option<DeadReckoning> {
        self.onboard
            .dead_reckoned
            .as_ref()
            .map(|solution| DeadReckoning {
                horizontal_1sigma_m: solution.integrity.horizontal_1sigma_m,
                vertical_1sigma_m: solution.integrity.vertical_1sigma_m,
                quality: solution.integrity.quality.into(),
            })
    }
}

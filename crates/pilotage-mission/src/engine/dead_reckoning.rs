//! Flight on an onboard solution beyond its limits: the plan goes on by
//! dead reckoning under [`crate::BeyondLimits::DeadReckon`], and the
//! engine says so with the uncertainty it flies on, so an operator sees a
//! navigator that has lost its position aid instead of a mission that
//! looks healthy.

use super::{MissionEngine, MissionEvent, NavQuality};

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

    /// Reports the dead reckoning of this tick: when it begins, each time
    /// its horizontal uncertainty doubles from the value last reported,
    /// and its end when a solution within the limits is flown again. A
    /// dead reckoning that ends in a hold is reported by the hold.
    pub(super) fn report_dead_reckoning(&mut self, flown: bool, events: &mut Vec<MissionEvent>) {
        let reported = self.onboard.reported_dead_reckoning_m;
        match (self.dead_reckoning(), reported) {
            (Some(now), last) => {
                let sigma = now.horizontal_1sigma_m;
                if last.is_none_or(|last| sigma >= 2.0 * last) {
                    self.onboard.reported_dead_reckoning_m = Some(sigma);
                    events.push(MissionEvent::DeadReckoning {
                        horizontal_1sigma_m: sigma,
                        vertical_1sigma_m: now.vertical_1sigma_m,
                    });
                }
            }
            (None, Some(_)) => {
                self.onboard.reported_dead_reckoning_m = None;
                if let Some(solution) = self.last_solution.as_ref().filter(|_| flown) {
                    events.push(MissionEvent::NavigationRecovered {
                        horizontal_1sigma_m: solution.integrity.horizontal_1sigma_m,
                    });
                }
            }
            (None, None) => {}
        }
    }
}

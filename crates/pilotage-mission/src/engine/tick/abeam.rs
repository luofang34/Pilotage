//! Sequencing the plan: on its path, or abeam of it under a selected
//! heading.
//!
//! A vehicle on a selected heading does not fly the plan's path, but its
//! plan keeps up with it, as a flight management system's does in a
//! heading mode: the active waypoint sequences when the vehicle goes
//! abeam of it near the route. Otherwise the return to the plan would
//! turn the vehicle back to a waypoint that is behind it.

use navigate_contract::NavigationSolution;
use navigate_route::SequenceReason;

use super::super::{MissionEngine, MissionEvent};

impl MissionEngine {
    /// Sequences the plan on one solution: as the path is flown, or, under
    /// a selected heading, when the vehicle goes abeam of the active
    /// waypoint within the configured bound of the active leg.
    pub(super) fn sequence_plan(
        &mut self,
        solution: &NavigationSolution,
        events: &mut Vec<MissionEvent>,
    ) {
        if !self.heading_selected() {
            return self.advance_plan(solution, events);
        }
        let bound_m = self.config.selected_heading_sequence_bound_m;
        if let Some((to_index, cross_track_m)) = self.route.pass_abeam(&solution.position, bound_m)
        {
            events.push(MissionEvent::LegAdvanced {
                to_index,
                reason: SequenceReason::Passed { cross_track_m },
            });
        }
    }
}

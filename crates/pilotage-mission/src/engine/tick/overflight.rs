//! A fix the vehicle flew past without capturing it. Navigate sequences a
//! fix only inside its capture radius, and track guidance keeps the leg's
//! course, so a fix missed by more than that radius would be followed
//! along its leg without end. Past the fix, the leg is flown direct to the
//! fix instead: the vehicle turns back and captures it.

use navigate_contract::{GeodeticPosition, Waypoint};
use navigate_geodesy::{along_track_m, distance_m};

use super::super::{MissionEngine, MissionEvent};

impl MissionEngine {
    /// The start of the track guidance follows toward `to`: the leg's
    /// start, or none (direct to the fix) once the vehicle is past the fix.
    /// The first tick past each fix reports it.
    pub(super) fn track_start(
        &mut self,
        position: &GeodeticPosition,
        from: Option<GeodeticPosition>,
        to: &Waypoint,
        events: &mut Vec<MissionEvent>,
    ) -> Option<GeodeticPosition> {
        let from = from?;
        if !overflown(position, &from, &to.position) {
            return Some(from);
        }
        self.counters.fix_overflown = self.counters.fix_overflown.wrapping_add(1);
        let to_index = self.execution.active_index();
        if self.overflown_fix != Some(to_index) {
            self.overflown_fix = Some(to_index);
            events.push(MissionEvent::FixOverflown { to_index });
        }
        None
    }
}

/// Whether `position` lies beyond `to` along the track `from` → `to`.
/// A track too short to have a direction is never overflown.
pub(super) fn overflown(
    position: &GeodeticPosition,
    from: &GeodeticPosition,
    to: &GeodeticPosition,
) -> bool {
    along_track_m(position, from, to).is_ok_and(|along| along > distance_m(from, to))
}

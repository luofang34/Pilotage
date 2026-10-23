//! A fix the vehicle flew past without capturing it. Navigate sequences a
//! fix only inside its capture radius, and track guidance keeps the leg's
//! course, so a fix missed by more than that radius would be followed
//! along its leg without end. Past the fix, the leg is flown direct to the
//! fix instead: the vehicle turns back and captures it.

use navigate_contract::GeodeticPosition;
use navigate_geodesy::{along_track_m, distance_m};

/// Whether `position` lies beyond `to` along the track `from` → `to`.
/// A track too short to have a direction is never overflown.
pub(super) fn overflown(
    position: &GeodeticPosition,
    from: &GeodeticPosition,
    to: &GeodeticPosition,
) -> bool {
    along_track_m(position, from, to).is_ok_and(|along| along > distance_m(from, to))
}

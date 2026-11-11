//! The horizontal origin of the flight controller's local frame, as the
//! flight controller last reported it.
//!
//! A consumer that knows where the local frame should be (the mission
//! anchor the host gave the flight controller) cross-checks it here, and
//! one that projects local positions follows a rebase by watching
//! [`GlobalOriginUpdate::rebases`]. The report carries no height this cache
//! keeps: its sea-level altitude cannot be compared with an ellipsoidal one.

use web_time::Instant;

use super::LinkState;

/// The latest horizontal origin of the flight controller's local frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalOriginUpdate {
    /// Latitude and longitude in degrees·1e7, as the wire carries them.
    pub lat_lon: [i32; 2],
    /// How many times the origin has moved since the first report on this
    /// link, wrapping. A repeated report of the same origin leaves it.
    pub rebases: u32,
    /// When the latest report of this origin arrived.
    pub received_at: Instant,
}

/// Folds one origin report into the cache.
///
/// A latitude and longitude both exactly zero is an origin the flight
/// controller has not set: 0,0 is a real place, so nothing downstream could
/// tell it from an origin there.
pub(super) fn apply(latest: &mut LinkState, lat_lon: [i32; 2], now: Instant) {
    if lat_lon == [0, 0] {
        return;
    }
    let rebases = match latest.global_origin {
        None => 0,
        Some(previous) if previous.lat_lon == lat_lon => previous.rebases,
        Some(previous) => previous.rebases.wrapping_add(1),
    };
    latest.global_origin = Some(GlobalOriginUpdate {
        lat_lon,
        rebases,
        received_at: now,
    });
}

#[cfg(test)]
mod tests;

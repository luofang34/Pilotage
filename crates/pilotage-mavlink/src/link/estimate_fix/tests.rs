//! The typed value both adapters build from one cached fix.

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Instant;

use pilotage_adapter_api::{MeasurementClock, SourceRole};

use super::estimate_geodetic_fix;
use crate::link::{GnssFixUpdate, LinkState};

fn fix(alt_ellipsoid_mm: i32, accuracy_mm: [u32; 2]) -> GnssFixUpdate {
    GnssFixUpdate {
        lat_lon: [473_977_419, 85_455_938],
        alt_ellipsoid_mm,
        accuracy_mm,
        sequence: 4,
        received_since_start_ns: 9_000_000,
        received_at: Instant::now(),
    }
}

/// The height is the one above the ellipsoid, which needs no separation
/// model, and the accuracy is the receiver's own.
#[test]
fn the_fix_carries_the_ellipsoidal_height_and_the_stated_accuracy() {
    let latest = LinkState::default();
    let sample = estimate_geodetic_fix(&fix(536_000, [1_250, 2_100]), &latest)
        .expect("a complete fix is publishable");

    assert!((sample.position.latitude_deg - 47.397_741_9).abs() < 1e-7);
    assert!((sample.position.longitude_deg - 8.545_593_8).abs() < 1e-7);
    assert!((sample.position.vertical.height_m - 536.0).abs() < 1e-6);
    assert_eq!(sample.quality.horizontal_mm, 1_250);
    assert_eq!(sample.quality.vertical_mm, 2_100);
}

/// A receiver's solution is not an oracle, and the message names no clock
/// this side can vouch for, so the stamp names the one that received it.
#[test]
fn the_fix_rides_the_estimate_role_on_a_clock_the_receiver_can_name() {
    let latest = LinkState::default();
    let sample =
        estimate_geodetic_fix(&fix(536_000, [1_250, 2_100]), &latest).expect("publishable");

    assert_eq!(sample.stamp.role, SourceRole::OperationalEstimate);
    assert_eq!(sample.stamp.clock, MeasurementClock::HostMonotonic);
    assert_eq!(sample.stamp.acquired_at_ns, 9_000_000);
    assert_eq!(sample.stamp.sequence, 4);
}

/// A height below the ellipsoid is ordinary — most of Europe sits there —
/// so a negative value is a position, not a refusal.
#[test]
fn a_height_below_the_ellipsoid_is_still_a_height() {
    let latest = LinkState::default();
    let sample = estimate_geodetic_fix(&fix(-50_000, [1_250, 2_100]), &latest)
        .expect("below the ellipsoid is a real place");
    assert!((sample.position.vertical.height_m - (-50.0)).abs() < 1e-6);
}

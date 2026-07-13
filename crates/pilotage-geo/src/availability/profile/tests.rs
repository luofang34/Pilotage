//! Tests for the explicit, checked availability profile.
#![allow(clippy::expect_used, clippy::panic)]

use super::{AvailabilityProfile, AvailabilityProfileId, SIMULATOR_PROFILE_ID};
use crate::availability::InputHealth;
use crate::error::GeoError;

#[test]
fn the_simulator_profile_matches_its_golden_limits() {
    // Independent golden literals, NOT the numbers simulator() is built from, so
    // a change to the profile is caught here instead of drifting undetected.
    let sim = AvailabilityProfile::simulator();
    assert_eq!(sim.id(), SIMULATOR_PROFILE_ID);
    assert_eq!(sim.version(), 1);
    assert_eq!(sim.fresh_age_ns(), 200_000_000);
    assert_eq!(sim.usable_age_ns(), 1_000_000_000);
    assert_eq!(sim.fresh_pos_mm(), 5_000);
    assert_eq!(sim.usable_pos_mm(), 50_000);
    assert_eq!(sim.fresh_att_mrad(), 10);
    assert_eq!(sim.usable_att_mrad(), 50);
}

#[test]
fn simulator_health_boundaries_are_pinned_to_golden_limits() {
    let sim = AvailabilityProfile::simulator();
    // Age: 200_000_000 ns fresh / 1_000_000_000 ns usable.
    assert_eq!(sim.age_health(200_000_000), InputHealth::Ok);
    assert_eq!(sim.age_health(200_000_001), InputHealth::Degraded);
    assert_eq!(sim.age_health(1_000_000_000), InputHealth::Degraded);
    assert_eq!(sim.age_health(1_000_000_001), InputHealth::Failed);
    // Position: 5_000 mm fresh / 50_000 mm usable.
    assert_eq!(sim.position_mm_health(5_000), InputHealth::Ok);
    assert_eq!(sim.position_mm_health(5_001), InputHealth::Degraded);
    assert_eq!(sim.position_mm_health(50_000), InputHealth::Degraded);
    assert_eq!(sim.position_mm_health(50_001), InputHealth::Failed);
    // Attitude: 10 mrad fresh / 50 mrad usable.
    assert_eq!(sim.attitude_mrad_health(10), InputHealth::Ok);
    assert_eq!(sim.attitude_mrad_health(11), InputHealth::Degraded);
    assert_eq!(sim.attitude_mrad_health(50), InputHealth::Degraded);
    assert_eq!(sim.attitude_mrad_health(51), InputHealth::Failed);
}

#[test]
fn profile_new_rejects_zero_and_non_monotonic_limits() {
    let id = AvailabilityProfileId(7);
    // A fresh age not strictly tighter than the usable age.
    assert_eq!(
        AvailabilityProfile::new(id, 1, 1_000, 1_000, 1, 2, 1, 2),
        Err(GeoError::InvalidAvailabilityProfile { field: "age" }),
    );
    // A zero fresh position limit.
    assert_eq!(
        AvailabilityProfile::new(id, 1, 1, 2, 0, 2, 1, 2),
        Err(GeoError::InvalidAvailabilityProfile { field: "position" }),
    );
    // A fresh attitude limit wider than the usable one.
    assert_eq!(
        AvailabilityProfile::new(id, 1, 1, 2, 1, 2, 9, 3),
        Err(GeoError::InvalidAvailabilityProfile { field: "attitude" }),
    );
    // A fully monotonic, non-zero set is accepted; its accessors reflect it.
    let ok = AvailabilityProfile::new(id, 4, 1, 2, 1, 2, 1, 2).expect("monotonic");
    assert_eq!(ok.id(), id);
    assert_eq!(ok.version(), 4);
    assert_eq!(ok.fresh_age_ns(), 1);
    assert_eq!(ok.usable_att_mrad(), 2);
}

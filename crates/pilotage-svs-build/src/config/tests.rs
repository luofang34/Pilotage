//! Unit tests for build-configuration validation.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::error::BuildError;
use crate::fixtures;

#[test]
fn valid_config_passes() {
    assert!(fixtures::config().validate().is_ok());
}

#[test]
fn non_positive_tile_size_is_refused() {
    let mut config = fixtures::config();
    config.params.tile_deg = 0.0;
    assert!(matches!(
        config.validate(),
        Err(BuildError::InvalidConfig { .. })
    ));
}

#[test]
fn degenerate_coverage_is_refused() {
    let mut config = fixtures::config();
    config.coverage.max_lat_deg = config.coverage.min_lat_deg;
    assert!(matches!(
        config.validate(),
        Err(BuildError::InvalidConfig { .. })
    ));
}

#[test]
fn misordered_effectivity_is_refused() {
    let mut config = fixtures::config();
    config.identity.effectivity.expiry = pilotage_svs_db::DayNumber(0);
    assert!(matches!(
        config.validate(),
        Err(BuildError::InvalidConfig { .. })
    ));
}

#[test]
fn inverted_elevation_bounds_are_refused() {
    let mut config = fixtures::config();
    config.params.elevation_min_m = 100.0;
    config.params.elevation_max_m = -100.0;
    assert!(matches!(
        config.validate(),
        Err(BuildError::InvalidConfig { .. })
    ));
}

//! Unit tests for the source model.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_svs_db::UseRestrictions;

use super::{LicenseCode, SourceId};
use crate::fixtures;

#[test]
fn license_maps_to_restrictions() {
    assert_eq!(LicenseCode::Open.restrictions(), UseRestrictions::NONE);
    assert_eq!(
        LicenseCode::NonOperational.restrictions(),
        UseRestrictions::NO_OPERATIONAL_USE
    );
    assert_eq!(
        LicenseCode::TrainingOnly.restrictions(),
        UseRestrictions::TRAINING_ONLY
    );
}

#[test]
fn terrain_grid_indexing_is_consistent() {
    let grid = fixtures::terrain_grid();
    assert_eq!(grid.post(0, 0), Some(100.0));
    assert_eq!(grid.post(1, 2), Some(100.0 + 10.0 + 2.0));
    assert_eq!(grid.post(grid.rows, 0), None, "out-of-range index is None");
}

#[test]
fn meta_lookup_finds_declared_sources() {
    let dataset = fixtures::dataset();
    assert!(dataset.meta_for(fixtures::TERRAIN_SRC).is_some());
    assert!(dataset.meta_for(SourceId(999)).is_none());
}

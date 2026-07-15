//! Tests for the semantic diff between builds.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pilotage_svs_db::PackageVersion;

use super::{SemanticDiff, TileChangeKind};
use crate::build_package;
use crate::fixtures;
use crate::source::{Obstacle, ObstacleKind, SourceRecordRef};

#[test]
fn identical_builds_diff_empty() {
    let a = build_package(&fixtures::config(), &fixtures::dataset()).expect("a");
    let b = build_package(&fixtures::config(), &fixtures::dataset()).expect("b");
    assert!(SemanticDiff::between(&a, &b).is_empty());
}

#[test]
fn changed_terrain_post_is_a_changed_tile() {
    let old = build_package(&fixtures::config(), &fixtures::dataset()).expect("old");
    let mut config = fixtures::config();
    config.identity.version = PackageVersion::new(1, 0, 1);
    let mut dataset = fixtures::dataset();
    dataset.terrain[0].posts[5] = Some(4242.0);
    let new = build_package(&config, &dataset).expect("new");
    let diff = SemanticDiff::between(&old, &new);
    assert!(!diff.is_empty());
    assert_eq!(diff.old_version, (1, 0, 0));
    assert_eq!(diff.new_version, (1, 0, 1));
    assert!(
        diff.tiles.iter().any(|t| t.kind == TileChangeKind::Changed),
        "a changed post must surface as a changed tile"
    );
}

#[test]
fn new_obstacle_tile_is_added_old_is_removed() {
    let base = build_package(&fixtures::config(), &fixtures::dataset()).expect("base");
    let mut dataset = fixtures::dataset();
    // Add an obstacle in a far tile of the coverage so a new obstacle tile appears.
    dataset.obstacles.push(Obstacle {
        lat_deg: 40.9,
        lon_deg: -74.1,
        height_m: 30.0,
        kind: ObstacleKind::Mast,
        source: SourceRecordRef::obstacle(fixtures::OBSTACLE_SRC, 1),
    });
    let grown = build_package(&fixtures::config(), &dataset).expect("grown");
    let added = SemanticDiff::between(&base, &grown);
    assert!(added.tiles.iter().any(|t| t.kind == TileChangeKind::Added));
    let removed = SemanticDiff::between(&grown, &base);
    assert!(
        removed
            .tiles
            .iter()
            .any(|t| t.kind == TileChangeKind::Removed)
    );
}

//! Tests for the data-quality report: outliers, invalid heights, clipping, and
//! merging, exercised end-to-end through the chain.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use crate::build_package;
use crate::fixtures;
use crate::provenance::Disposition;
use crate::source::{Obstacle, ObstacleKind, SourceRecordRef};

fn extra_obstacle(record: u32, lat: f64, lon: f64, height: f64, kind: ObstacleKind) -> Obstacle {
    Obstacle {
        lat_deg: lat,
        lon_deg: lon,
        height_m: height,
        kind,
        source: SourceRecordRef::obstacle(fixtures::OBSTACLE_SRC, record),
    }
}

fn count_outliers(dispositions: &[crate::provenance::RecordDisposition]) -> usize {
    dispositions
        .iter()
        .filter(|d| matches!(d.disposition, Disposition::RejectedOutlier { .. }))
        .count()
}

#[test]
fn out_of_bounds_terrain_post_is_rejected() {
    let mut dataset = fixtures::dataset();
    dataset.terrain[0].posts[5] = Some(1.0e9);
    let artifact = build_package(&fixtures::config(), &dataset).expect("build");
    assert!(artifact.reports.quality.outliers_rejected >= 1);
    assert!(count_outliers(&artifact.provenance.dispositions) >= 1);
}

#[test]
fn invalid_obstacle_heights_are_rejected() {
    let mut dataset = fixtures::dataset();
    dataset.obstacles = vec![
        extra_obstacle(0, 40.2, -74.7, 0.0, ObstacleKind::Tower),
        extra_obstacle(1, 40.2, -74.6, -5.0, ObstacleKind::Tower),
        extra_obstacle(2, 40.2, -74.5, f64::NAN, ObstacleKind::Tower),
        extra_obstacle(3, 40.2, -74.4, 1.0e9, ObstacleKind::Tower),
    ];
    let artifact = build_package(&fixtures::config(), &dataset).expect("build");
    assert_eq!(
        artifact.reports.quality.outliers_rejected, 4,
        "every invalid obstacle height must be rejected"
    );
    assert_eq!(artifact.reports.coverage.obstacles, 0);
}

#[test]
fn obstacle_outside_coverage_is_clipped() {
    let mut dataset = fixtures::dataset();
    dataset.obstacles = vec![extra_obstacle(0, 50.0, -74.7, 50.0, ObstacleKind::Tower)];
    let artifact = build_package(&fixtures::config(), &dataset).expect("build");
    assert_eq!(artifact.reports.quality.clipped, 1);
    assert!(
        artifact
            .provenance
            .dispositions
            .iter()
            .any(|d| matches!(d.disposition, Disposition::Clipped))
    );
}

#[test]
fn co_located_obstacles_merge_keeping_tallest() {
    let mut dataset = fixtures::dataset();
    dataset.obstacles = vec![
        extra_obstacle(0, 40.2000, -74.7000, 50.0, ObstacleKind::Tower),
        extra_obstacle(1, 40.2005, -74.7005, 60.0, ObstacleKind::Tower),
    ];
    let artifact = build_package(&fixtures::config(), &dataset).expect("build");
    assert_eq!(artifact.reports.quality.obstacles_merged, 1);
    assert_eq!(artifact.reports.coverage.obstacles, 1);
    assert!(
        artifact
            .provenance
            .dispositions
            .iter()
            .any(|d| matches!(d.disposition, Disposition::Merged { .. }))
    );
}

#[test]
fn distinct_kinds_do_not_merge() {
    let mut dataset = fixtures::dataset();
    dataset.obstacles = vec![
        extra_obstacle(0, 40.2000, -74.7000, 50.0, ObstacleKind::Tower),
        extra_obstacle(1, 40.2001, -74.7001, 60.0, ObstacleKind::Mast),
    ];
    let artifact = build_package(&fixtures::config(), &dataset).expect("build");
    assert_eq!(artifact.reports.quality.obstacles_merged, 0);
    assert_eq!(artifact.reports.coverage.obstacles, 2);
}

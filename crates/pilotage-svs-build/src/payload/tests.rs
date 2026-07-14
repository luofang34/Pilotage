//! Unit tests for the tile payload encoders.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::{encode_obstacles, encode_terrain};
use crate::element::{OutputObstacle, OutputPost};
use crate::source::{ObstacleKind, SourceId, SourceRecordRef};

fn post(i: u32, j: u32, elev: f64) -> OutputPost {
    OutputPost {
        i,
        j,
        lat_deg: 0.0,
        lon_deg: 0.0,
        elevation_m: elev,
        sources: vec![SourceRecordRef {
            source: SourceId(1),
            record: i * 10 + j,
        }],
    }
}

#[test]
fn terrain_encoding_is_order_independent() {
    let ordered = vec![post(0, 0, 1.0), post(0, 1, 2.0), post(1, 0, 3.0)];
    let shuffled = vec![post(1, 0, 3.0), post(0, 0, 1.0), post(0, 1, 2.0)];
    assert_eq!(
        encode_terrain(&ordered),
        encode_terrain(&shuffled),
        "payload bytes must not depend on element order"
    );
}

#[test]
fn terrain_encoding_reflects_content() {
    let a = vec![post(0, 0, 1.0)];
    let b = vec![post(0, 0, 2.0)];
    assert_ne!(encode_terrain(&a), encode_terrain(&b));
}

fn obstacle(lat: f64, lon: f64, kind: ObstacleKind) -> OutputObstacle {
    OutputObstacle {
        lat_deg: lat,
        lon_deg: lon,
        height_m: 10.0,
        kind,
        sources: vec![SourceRecordRef {
            source: SourceId(2),
            record: 0,
        }],
    }
}

#[test]
fn obstacle_encoding_is_order_independent() {
    let a = vec![
        obstacle(1.0, 2.0, ObstacleKind::Tower),
        obstacle(1.0, 2.0, ObstacleKind::Mast),
    ];
    let b = vec![
        obstacle(1.0, 2.0, ObstacleKind::Mast),
        obstacle(1.0, 2.0, ObstacleKind::Tower),
    ];
    assert_eq!(encode_obstacles(&a), encode_obstacles(&b));
}

#[test]
fn terrain_payload_carries_domain_magic() {
    let bytes = encode_terrain(&[post(0, 0, 1.0)]);
    assert_eq!(&bytes[..8], b"SVSBTERR");
}

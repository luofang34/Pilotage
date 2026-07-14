//! Integration tests for the chain: build + verify, reproducibility, fail-closed
//! behavior, seams, holes, datum conversion, boundaries, and the independent
//! oracle.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use ed25519_dalek::SigningKey;
use pilotage_geo::{GeoidModelId, HorizontalDatum, VerticalDatum};
use pilotage_svs_db::{DayNumber, FeatureClass, TrustAnchor, TrustRoot, UsePolicy, verify_package};

use crate::canonical_package_bytes;
use crate::chain::{build_package, geo_tile_for};
use crate::fixtures;
use crate::provenance::RecordKey;
use crate::source::{LicenseCode, Obstacle, ObstacleKind, SourceId, SourceRecordRef};

/// A trust root that trusts the fixture signing key.
fn trust_root() -> TrustRoot {
    let config = fixtures::config();
    let key = SigningKey::from_bytes(&config.signing.signing_seed);
    TrustRoot::new(vec![TrustAnchor {
        key_id: config.signing.key_id,
        public_key: key.verifying_key().to_bytes(),
    }])
}

#[test]
fn builds_and_verifies_against_svs02() {
    let config = fixtures::config();
    let artifact = build_package(&config, &fixtures::dataset()).expect("build succeeds");
    let verified = verify_package(
        &artifact.package,
        &trust_root(),
        DayNumber(150),
        None,
        UsePolicy::SimulatorPermitted,
    )
    .expect("SVS-02 verifier accepts the built package");
    assert_eq!(
        verified.manifest().tile_count as usize,
        artifact.package.tiles.len()
    );
    let features = artifact.package.manifest.content.features;
    for class in [
        FeatureClass::Terrain,
        FeatureClass::Obstacles,
        FeatureClass::Aerodromes,
        FeatureClass::Runways,
    ] {
        assert!(features.contains(class), "missing class {class:?}");
    }
    assert!(artifact.package.manifest.simulation_only);
}

#[test]
fn build_is_byte_identical_across_runs() {
    let config = fixtures::config();
    let a = build_package(&config, &fixtures::dataset()).expect("build a");
    let b = build_package(&config, &fixtures::dataset()).expect("build b");
    assert_eq!(
        canonical_package_bytes(&a.package),
        canonical_package_bytes(&b.package),
        "signed package bytes must be byte-identical"
    );
    assert_eq!(
        a.package.manifest.signature.bytes, b.package.manifest.signature.bytes,
        "signature over identical content must be identical"
    );
    assert_eq!(
        serde_json::to_vec(&a.provenance).unwrap(),
        serde_json::to_vec(&b.provenance).unwrap(),
        "provenance must reproduce byte-identically"
    );
    assert_eq!(
        serde_json::to_vec(&a.reports).unwrap(),
        serde_json::to_vec(&b.reports).unwrap(),
        "reports must reproduce byte-identically"
    );
    assert_eq!(
        a.bundle_signature, b.bundle_signature,
        "the provenance+report bundle signature must reproduce identically"
    );
}

#[test]
fn unknown_source_datum_fails_closed_with_no_package() {
    let mut dataset = fixtures::dataset();
    dataset.meta[0].vertical_datum = VerticalDatum::Unknown;
    let result = build_package(&fixtures::config(), &dataset);
    assert!(
        matches!(result, Err(crate::BuildError::UnknownSourceDatum { .. })),
        "a stage failure must abort with a typed error and no package: {result:?}"
    );
}

#[test]
fn empty_output_fails_closed() {
    let config = fixtures::config();
    let empty = crate::source::SourceDataset::default();
    let result = build_package(&config, &empty);
    assert!(matches!(result, Err(crate::BuildError::EmptyOutput)));
}

#[test]
fn seam_partition_holds_over_multiple_tiles() {
    let artifact = build_package(&fixtures::config(), &fixtures::dataset()).expect("build");
    let reports = &artifact.reports;
    assert!(reports.seam.ok, "every node must land in exactly one tile");
    assert_eq!(reports.seam.conflicts, 0);
    assert_eq!(
        reports.coverage.covered_nodes + reports.coverage.void_nodes,
        reports.coverage.total_nodes,
        "every node accounted for exactly once"
    );
    assert!(
        reports.coverage.terrain_tiles >= 4,
        "coverage spans multiple tiles"
    );
}

#[test]
fn wide_hole_stays_void_and_is_reported() {
    let mut dataset = fixtures::dataset();
    // Void columns 1..cols: the run reaches the row's right edge with no bounding
    // post, so the hole policy cannot bridge it and every node stays void.
    let (rows, cols) = (dataset.terrain[0].rows, dataset.terrain[0].cols);
    for r in 0..rows {
        for c in 1..cols {
            dataset.terrain[0].posts[(r * cols + c) as usize] = None;
        }
    }
    let artifact = build_package(&fixtures::config(), &dataset).expect("build with holes");
    assert!(
        !artifact.reports.hole.voids.is_empty(),
        "an unbridgeable void must be reported as a hole"
    );
    assert!(artifact.reports.coverage.void_nodes > 0);
    assert!(artifact.reports.coverage.covered_nodes < artifact.reports.coverage.total_nodes);
}

#[test]
fn narrow_hole_is_filled_within_policy() {
    let mut dataset = fixtures::dataset();
    // Void a single interior post, bracketed on both sides within max_hole_span.
    let cols = dataset.terrain[0].cols;
    let idx = (cols + 1) as usize;
    dataset.terrain[0].posts[idx] = None;
    let full = build_package(&fixtures::config(), &fixtures::dataset()).expect("full");
    let filled = build_package(&fixtures::config(), &dataset).expect("filled");
    assert_eq!(
        filled.reports.coverage.covered_nodes, full.reports.coverage.covered_nodes,
        "a within-policy hole is filled, leaving no void"
    );
}

#[test]
fn vertical_datum_conversion_shifts_terrain() {
    // Source MSL (geoid declared) -> target ellipsoid: elevations shift by N.
    let mut dataset = fixtures::dataset();
    dataset.meta[0].vertical_datum = VerticalDatum::Msl;
    dataset.meta[0].geoid = GeoidModelId(7);
    let artifact = build_package(&fixtures::config(), &dataset).expect("msl build");
    let posts = decode_terrain_all(&artifact);
    let (i, j, elev) = posts[0];
    let oracle = oracle_elevation_msl(i, j);
    assert!(
        (elev - oracle).abs() < 1e-9,
        "converted elevation {elev} must match the independent MSL oracle {oracle}"
    );
}

#[test]
fn terrain_matches_independent_bilinear_oracle() {
    let artifact = build_package(&fixtures::config(), &fixtures::dataset()).expect("build");
    let posts = decode_terrain_all(&artifact);
    assert!(!posts.is_empty());
    for (i, j, elev) in posts {
        let oracle = oracle_elevation_ellipsoid(i, j);
        assert!(
            (elev - oracle).abs() < 1e-9,
            "node ({i},{j}) elevation {elev} disagrees with oracle {oracle}"
        );
    }
}

#[test]
fn anti_meridian_position_wraps_into_coverage() {
    let mut config = fixtures::config();
    config.coverage.min_lon_deg = -180.0;
    config.coverage.max_lon_deg = -179.0;
    config.coverage.min_lat_deg = 40.0;
    config.coverage.max_lat_deg = 41.0;
    // +180 longitude normalizes to -180, which is inside the box (inclusive low).
    let tile = geo_tile_for(&config, 99, 40.5, 180.0).expect("antimeridian tiles");
    let wrapped = geo_tile_for(&config, 99, 40.5, -180.0).expect("wrapped tiles");
    assert_eq!(
        (tile.lat_index, tile.lon_index),
        (wrapped.lat_index, wrapped.lon_index),
        "+180 and -180 must tile identically"
    );
}

#[test]
fn polar_position_tiles_without_error() {
    let config = fixtures::config();
    let tile = geo_tile_for(&config, 99, 90.0, 10.0).expect("pole tiles");
    assert_eq!(
        tile.lat_index,
        (90.0f64 / config.params.tile_deg).floor() as i32
    );
}

/// Decodes every terrain tile payload into `(i, j, elevation)` triples.
fn decode_terrain_all(artifact: &super::BuildArtifact) -> Vec<(u32, u32, f64)> {
    let mut out = Vec::new();
    for tile in &artifact.package.tiles {
        if tile.key.class == FeatureClass::Terrain {
            out.extend(decode_terrain(&tile.bytes));
        }
    }
    out.sort_by_key(|(i, j, _)| (*i, *j));
    out
}

/// Decodes one terrain tile payload (see `payload::encode_terrain`).
fn decode_terrain(bytes: &[u8]) -> Vec<(u32, u32, f64)> {
    let mut out = Vec::new();
    let count = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let mut off = 16usize;
    for _ in 0..count {
        let i = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let j = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        let elev = f64::from_bits(u64::from_le_bytes(
            bytes[off + 8..off + 16].try_into().unwrap(),
        ));
        out.push((i, j, elev));
        off += 16;
    }
    out
}

/// Independent bilinear elevation at output node `(i, j)` over the fixture grid,
/// computed from scratch with no shared code path.
fn oracle_elevation_ellipsoid(i: u32, j: u32) -> f64 {
    let lat = 40.0 + f64::from(i) * 0.25;
    let lon = -75.0 + f64::from(j) * 0.25;
    let fr = (lat - 39.5) / 0.5;
    let fc = (lon - (-75.5)) / 0.5;
    let r0 = fr.floor();
    let c0 = fc.floor();
    let tr = fr - r0;
    let tc = fc - c0;
    let h = |r: f64, c: f64| 100.0 + r * 10.0 + c;
    let top = h(r0, c0) * (1.0 - tc) + h(r0, c0 + 1.0) * tc;
    let bottom = h(r0 + 1.0, c0) * (1.0 - tc) + h(r0 + 1.0, c0 + 1.0) * tc;
    top * (1.0 - tr) + bottom * tr
}

/// Independent MSL->ellipsoid oracle: bilinear over posts already converted at
/// each corner's coordinates, matching the chain's convert-then-interpolate.
fn oracle_elevation_msl(i: u32, j: u32) -> f64 {
    let lat = 40.0 + f64::from(i) * 0.25;
    let lon = -75.0 + f64::from(j) * 0.25;
    let fr = (lat - 39.5) / 0.5;
    let fc = (lon - (-75.5)) / 0.5;
    let r0 = fr.floor();
    let c0 = fc.floor();
    let tr = fr - r0;
    let tc = fc - c0;
    let corner = |r: f64, c: f64| {
        let clat = 39.5 + r * 0.5;
        let clon = -75.5 + c * 0.5;
        let msl = 100.0 + r * 10.0 + c;
        msl + crate::datum::geoid_separation_m(clat, clon)
    };
    let top = corner(r0, c0) * (1.0 - tc) + corner(r0, c0 + 1.0) * tc;
    let bottom = corner(r0 + 1.0, c0) * (1.0 - tc) + corner(r0 + 1.0, c0 + 1.0) * tc;
    top * (1.0 - tr) + bottom * tr
}

#[test]
fn overlapping_terrain_grids_resolve_deterministically() {
    // A second grid overlaps the first over the whole coverage, with heights
    // offset by 1000 so which grid wins is observable.
    let mut overlay = fixtures::terrain_grid();
    overlay.source = SourceId(4);
    for h in overlay.posts.iter_mut().flatten() {
        *h += 1000.0;
    }
    let mut natural = fixtures::dataset();
    natural
        .meta
        .push(fixtures::meta(SourceId(4), LicenseCode::Open));
    natural.terrain.push(overlay.clone());
    let mut swapped = fixtures::dataset();
    swapped
        .meta
        .push(fixtures::meta(SourceId(4), LicenseCode::Open));
    swapped.terrain.insert(0, overlay);
    let a = build_package(&fixtures::config(), &natural).expect("natural order");
    let b = build_package(&fixtures::config(), &swapped).expect("swapped order");
    assert_eq!(
        canonical_package_bytes(&a.package),
        canonical_package_bytes(&b.package),
        "overlap resolution must not depend on input order"
    );
    assert!(
        decode_terrain_all(&a).iter().all(|(_, _, e)| *e < 500.0),
        "the lower-id source wins the overlap, so heights stay in the base range"
    );
}

#[test]
fn record_lineage_resolves_a_post_to_its_source_corners() {
    let artifact = build_package(&fixtures::config(), &fixtures::dataset()).expect("build");
    // Node (0,0) is at lat 40.0, lon -75.0. Over the source grid (origin
    // 39.5,-75.5, step 0.5, 4 cols) it brackets source cells (1,1),(1,2),(2,1),
    // (2,2), i.e. record indices 5,6,9,10 of the terrain source.
    let record = artifact
        .provenance
        .records
        .iter()
        .find(|r| r.key == RecordKey::TerrainNode { i: 0, j: 0 })
        .expect("a lineage entry for node (0,0)");
    let expected: Vec<SourceRecordRef> = [5, 6, 9, 10]
        .into_iter()
        .map(|record| SourceRecordRef {
            source: fixtures::TERRAIN_SRC,
            record,
        })
        .collect();
    assert_eq!(
        record.sources, expected,
        "an individual output post must resolve to its exact source corners"
    );
}

#[test]
fn record_lineage_of_a_merged_obstacle_lists_every_source() {
    let mut dataset = fixtures::dataset();
    dataset.obstacles = vec![
        Obstacle {
            lat_deg: 40.2000,
            lon_deg: -74.7000,
            height_m: 50.0,
            kind: ObstacleKind::Tower,
            source: SourceRecordRef {
                source: fixtures::OBSTACLE_SRC,
                record: 0,
            },
        },
        Obstacle {
            lat_deg: 40.2005,
            lon_deg: -74.7005,
            height_m: 60.0,
            kind: ObstacleKind::Tower,
            source: SourceRecordRef {
                source: fixtures::OBSTACLE_SRC,
                record: 1,
            },
        },
    ];
    let artifact = build_package(&fixtures::config(), &dataset).expect("build");
    let record = artifact
        .provenance
        .records
        .iter()
        .find(|r| r.class == pilotage_svs_db::FeatureClass::Obstacles.to_u8())
        .expect("an obstacle record lineage");
    assert_eq!(
        record.sources,
        vec![
            SourceRecordRef {
                source: fixtures::OBSTACLE_SRC,
                record: 0,
            },
            SourceRecordRef {
                source: fixtures::OBSTACLE_SRC,
                record: 1,
            },
        ],
        "a merged obstacle must trace to every source that merged into it"
    );
}

#[test]
fn horizontal_datum_datum_is_recorded_in_manifest() {
    let artifact = build_package(&fixtures::config(), &fixtures::dataset()).expect("build");
    assert_eq!(
        artifact.package.manifest.coverage.horizontal_datum,
        HorizontalDatum::Wgs84
    );
}

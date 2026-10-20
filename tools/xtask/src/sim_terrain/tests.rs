//! The simulator terrain plan, its tiles, and the archive it packs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;

use pilotage_terrain_query::TerrainArchive;

use super::{N51_PLAN, archive_path, tile_of, tiles, write_archive};

/// The N51 threshold, 40.5771861 N 74.7381918 W.
const THRESHOLD: (f64, f64) = (40.577_186_1, -74.738_191_8);

#[test]
fn the_threshold_is_in_the_tile_its_elevation_was_sampled_from() {
    // 56.3 m came from terrarium/15/9581/12336.png.
    assert_eq!(tile_of(15, THRESHOLD.0, THRESHOLD.1), (9581, 12336));
    assert_eq!(tile_of(0, THRESHOLD.0, THRESHOLD.1), (0, 0));
}

#[test]
fn the_n51_plan_holds_every_zoom_over_the_threshold_and_a_box_at_depth() {
    let tiles = tiles(&N51_PLAN);
    for zoom in 0..=15 {
        let (x, y) = tile_of(zoom, THRESHOLD.0, THRESHOLD.1);
        assert!(
            tiles.contains(&(zoom, x, y)),
            "zoom {zoom} misses the threshold"
        );
    }
    let z15 = tiles.iter().filter(|(zoom, _, _)| *zoom == 15).count();
    assert!((40..=80).contains(&z15), "{z15} tiles at zoom 15");
    assert!(tiles.len() < 150, "{} tiles in all", tiles.len());
    let mut sorted = tiles.clone();
    sorted.dedup();
    assert_eq!(sorted.len(), tiles.len());
}

fn terrarium_png(elevation_m: f64) -> Vec<u8> {
    let code = ((elevation_m + 32_768.0) * 256.0).round() as u32;
    let pixel = [(code >> 16) as u8, (code >> 8) as u8, code as u8];
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(Cursor::new(&mut bytes), 256, 256);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&pixel.repeat(256 * 256)).unwrap();
    writer.finish().unwrap();
    bytes
}

#[test]
fn the_packed_archive_answers_the_threshold_from_its_deepest_tile() {
    let root = std::env::temp_dir().join(format!("plt_xt_sim_terrain_{}", std::process::id()));
    std::fs::create_dir_all(root.join("target/sim-terrain")).unwrap();
    let archive = archive_path(&root, "n51");
    let deep = tile_of(15, THRESHOLD.0, THRESHOLD.1);
    let coarse = tile_of(10, THRESHOLD.0, THRESHOLD.1);
    write_archive(
        &archive,
        &N51_PLAN,
        &[
            ((10, coarse.0, coarse.1), terrarium_png(40.0)),
            ((15, deep.0, deep.1), terrarium_png(56.25)),
        ],
    )
    .unwrap();
    let mut terrain = TerrainArchive::open_blocking(&archive).unwrap();
    let sample = terrain
        .elevation_sample_blocking(THRESHOLD.0, THRESHOLD.1)
        .unwrap()
        .unwrap();
    assert_eq!(sample.zoom, 15);
    assert!((sample.elevation_m - 56.25).abs() < 1.0 / 256.0);
    let (name, version) = terrain.identity_blocking().unwrap();
    assert_eq!(
        (name.as_str(), version.as_deref()),
        ("Pilotage n51 simulator terrain", Some("1"))
    );
    // Rewriting replaces the archive whole.
    write_archive(
        &archive,
        &N51_PLAN,
        &[((10, coarse.0, coarse.1), terrarium_png(40.0))],
    )
    .unwrap();
    let mut terrain = TerrainArchive::open_blocking(&archive).unwrap();
    let sample = terrain
        .elevation_sample_blocking(THRESHOLD.0, THRESHOLD.1)
        .unwrap()
        .unwrap();
    assert_eq!(sample.zoom, 10);
    std::fs::remove_dir_all(&root).ok();
}

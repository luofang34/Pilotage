use std::path::PathBuf;

use super::{HttpTiles, write_atomically};
use crate::config::ESRI_WORLD_IMAGERY;
use crate::tiles::{TILE_SIZE, TileKey, TileSource};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pilotage-sim-tiles-test-{name}-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn png_tile(rgb: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(TILE_SIZE, TILE_SIZE, image::Rgb(rgb));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("a PNG encodes");
    bytes
}

#[test]
fn the_esri_template_puts_the_row_before_the_column() {
    let tiles = HttpTiles::cache_only(ESRI_WORLD_IMAGERY.to_owned(), scratch("url"));
    assert!(
        tiles
            .url(TileKey {
                z: 19,
                x: 10,
                y: 20
            })
            .ends_with("/tile/19/20/10")
    );
}

#[test]
fn a_cached_tile_loads_and_a_missing_tile_is_remembered() {
    let dir = scratch("cache");
    let key = TileKey { z: 17, x: 3, y: 4 };
    let path = dir.join("17").join("3").join("4.img");
    write_atomically(&path, &png_tile([10, 200, 30])).expect("the cache writes");
    let mut tiles = HttpTiles::cache_only(ESRI_WORLD_IMAGERY.to_owned(), dir.clone());
    let tile = tiles.tile(key).expect("the cached tile loads");
    assert_eq!(tile.pixel(5, 5), [10, 200, 30]);
    let other = TileKey { z: 17, x: 3, y: 5 };
    assert!(tiles.tile(other).is_none());
    assert!(tiles.missing.contains(&other));
    // A file that arrives later is not read again in this process.
    write_atomically(
        &dir.join("17").join("3").join("5.img"),
        &png_tile([1, 2, 3]),
    )
    .expect("the cache writes");
    assert!(tiles.tile(other).is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_wrong_size_tile_is_refused() {
    let dir = scratch("size");
    let image = image::RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 0]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("a PNG encodes");
    write_atomically(&dir.join("1").join("0").join("0.img"), &bytes).expect("the cache writes");
    let mut tiles = HttpTiles::cache_only(ESRI_WORLD_IMAGERY.to_owned(), dir.clone());
    assert!(tiles.tile(TileKey { z: 1, x: 0, y: 0 }).is_none());
    std::fs::remove_dir_all(&dir).ok();
}

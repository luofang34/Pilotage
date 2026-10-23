use super::{TileKey, locate, mercator_pixel, zoom_for_resolution};

#[test]
fn the_origin_tile_holds_the_null_island() {
    let (key, px, py) = locate(0.0, 0.0, 1);
    assert_eq!(key, TileKey { z: 1, x: 1, y: 1 });
    assert_eq!((px, py), (0, 0));
}

#[test]
fn princeton_is_in_the_expected_zoom_19_tile() {
    // Nassau Hall, Princeton, NJ.
    let (key, _, _) = locate(40.3487, -74.6593, 19);
    let [x, y] = mercator_pixel(40.3487, -74.6593, 19);
    assert_eq!(key.x, (x / 256.0) as u32);
    assert_eq!(key.y, (y / 256.0) as u32);
    assert_eq!((key.x, key.y), (153_413, 197_819));
}

#[test]
fn zoom_follows_the_ground_resolution() {
    // A 0.26 m pixel at 40 degrees north needs zoom 19.
    assert_eq!(zoom_for_resolution(0.26, 40.33, 19), 19);
    assert_eq!(zoom_for_resolution(0.05, 40.33, 19), 19);
    assert_eq!(zoom_for_resolution(2.0, 40.33, 19), 16);
    assert_eq!(zoom_for_resolution(f64::NAN, 40.33, 19), 19);
}

#[test]
fn a_parent_holds_its_children() {
    let key = TileKey { z: 5, x: 9, y: 13 };
    assert_eq!(key.parent(), Some(TileKey { z: 4, x: 4, y: 6 }));
    assert_eq!(TileKey { z: 0, x: 0, y: 0 }.parent(), None);
}

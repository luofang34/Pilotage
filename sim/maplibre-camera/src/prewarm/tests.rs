#![allow(clippy::expect_used, clippy::panic)]

use super::{square, tile_at, urls};
use crate::args::parse;

#[test]
fn the_site_tile_is_where_the_site_is() {
    // Solberg-Hunterdon (N51) at zoom 16 and 12.
    assert_eq!(tile_at(40.577_186, -74.738_192, 16), (19_162, 24_672));
    assert_eq!(tile_at(40.577_186, -74.738_192, 12), (1_197, 1_542));
    assert_eq!(square(40.577_186, -74.738_192, 16).len(), 81);
    // At the world's corner the square is clipped, not wrapped.
    assert_eq!(square(85.0, -180.0, 3).len(), 25);
}

#[test]
fn the_area_is_bounded_and_covers_every_source_to_its_deepest_zoom() {
    let config = parse(&["--host-port".to_owned(), "1".to_owned()]).expect("defaults");
    let urls = urls(&config, 40.577_186, -74.738_192);
    // Base zooms 12 to 16, detail zooms 17 to 19, terrain zooms 10 to 15,
    // 81 tiles each.
    assert_eq!(urls.len(), 14 * 81);
    let detail = urls
        .iter()
        .filter(|url| url.contains("exportImage"))
        .count();
    assert_eq!(detail, 3 * 81);
    assert!(urls.iter().any(|url| url.contains("/MapServer/tile/16/")));
    assert!(!urls.iter().any(|url| url.contains("/MapServer/tile/17/")));
    assert!(
        urls.iter().all(|url| !url.contains('{')),
        "every placeholder filled"
    );
    assert!(urls.iter().any(|url| url.contains("/terrarium/15/")));
    assert!(!urls.iter().any(|url| url.contains("/terrarium/16/")));
}

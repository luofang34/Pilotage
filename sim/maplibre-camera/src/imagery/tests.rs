#![allow(clippy::expect_used, clippy::panic)]

use super::ImageryRouter;
use crate::args::parse;

fn router() -> ImageryRouter {
    let config = parse(&["--host-port".to_owned(), "1".to_owned()]).expect("defaults");
    ImageryRouter::new(&config, "/nonexistent-cache")
}

#[test]
fn a_tile_past_the_base_zooms_comes_from_the_detail_server() {
    let router = router();
    let base = router
        .upstream("camera-imagery://16/19162/24672")
        .expect("routed")
        .expect("in the world");
    assert_eq!(
        base,
        "https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryOnly/MapServer/tile/16/24672/19162"
    );
    let detail = router
        .upstream("camera-imagery://17/38324/49344")
        .expect("routed")
        .expect("in the world");
    assert!(detail.contains("exportImage?bbox="), "{detail}");
    assert!(!detail.contains('{'), "{detail}");
}

#[test]
fn other_urls_pass_through_and_a_tile_off_the_world_is_none() {
    let router = router();
    assert_eq!(
        router.upstream("https://s3.amazonaws.com/elevation-tiles-prod/terrarium/12/1197/1542.png"),
        None
    );
    assert_eq!(router.upstream("camera-imagery://2/9/0"), Some(None));
    assert_eq!(router.upstream("camera-imagery://two/0/0"), None);
}

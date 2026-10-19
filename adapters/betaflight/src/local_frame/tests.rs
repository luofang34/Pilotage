#![allow(clippy::expect_used, clippy::panic)]

use super::LocalFrame;

#[test]
fn a_degree_has_the_wgs84_length_at_the_demo_origin() {
    let frame = LocalFrame::anchored_at(47.397_742, 8.545_594);
    let [north, _] = frame.north_east(48.397_742, 8.545_594);
    let [_, east] = frame.north_east(47.397_742, 9.545_594);
    assert!((north - 111_180.0).abs() < 50.0, "{north}");
    assert!((east - 75_400.0).abs() < 100.0, "{east}");
    assert_eq!(frame.north_east(47.397_742, 8.545_594), [0.0, 0.0]);
}

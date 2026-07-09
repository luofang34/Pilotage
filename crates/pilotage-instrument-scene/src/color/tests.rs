#![allow(clippy::expect_used, clippy::panic)]

use super::Rgba8;

#[test]
fn u32_round_trip_preserves_channels() {
    let c = Rgba8::rgba(1, 2, 3, 4);
    assert_eq!(Rgba8::from_u32(c.to_u32()), c);
}

#[test]
fn rgb_is_opaque() {
    assert_eq!(Rgba8::rgb(9, 8, 7).a, 255);
}

#[test]
fn packing_is_byte_ordered() {
    // Written little-endian, 0xAABBGGRR reads r,g,b,a in the byte stream.
    let c = Rgba8::rgba(0x11, 0x22, 0x33, 0x44);
    assert_eq!(c.to_u32(), 0x4433_2211);
}

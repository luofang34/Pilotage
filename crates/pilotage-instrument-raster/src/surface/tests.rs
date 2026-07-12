#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::error::RasterError;

fn dims(w: u32, h: u32) -> FramebufferDims {
    FramebufferDims::tight(w, h)
}

#[test]
fn rejects_zero_dimensions() {
    let mut buf = [0u8; 16];
    assert_eq!(
        Surface::new(&mut buf, dims(0, 4)).err(),
        Some(RasterError::ZeroFramebuffer)
    );
    assert_eq!(
        Surface::new(&mut buf, dims(4, 0)).err(),
        Some(RasterError::ZeroFramebuffer)
    );
}

#[test]
fn rejects_oversized_dimensions() {
    let mut buf = [0u8; 16];
    let over = crate::MAX_DIMENSION + 1;
    assert!(matches!(
        Surface::new(&mut buf, dims(over, 1)),
        Err(RasterError::FramebufferTooLarge { .. })
    ));
}

#[test]
fn rejects_short_stride_and_slice() {
    let mut buf = [0u8; 64];
    assert!(matches!(
        Surface::new(
            &mut buf,
            FramebufferDims {
                width: 4,
                height: 4,
                stride_bytes: 8,
            }
        ),
        Err(RasterError::StrideTooSmall { .. })
    ));
    let mut small = [0u8; 15];
    assert!(matches!(
        Surface::new(&mut small, dims(2, 2)),
        Err(RasterError::FramebufferTooSmall { .. })
    ));
}

#[test]
fn opaque_source_overwrites() {
    let mut buf = [9u8; 16];
    let mut s = Surface::new(&mut buf, dims(2, 2)).expect("surface");
    s.composite(0, 0, [10, 20, 30, 255]);
    assert_eq!(&buf[0..4], &[10, 20, 30, 255]);
}

#[test]
fn transparent_source_is_a_no_op() {
    let mut buf = [7u8; 16];
    let mut s = Surface::new(&mut buf, dims(2, 2)).expect("surface");
    s.composite(1, 1, [1, 2, 3, 0]);
    assert_eq!(&buf[12..16], &[7, 7, 7, 7]);
}

#[test]
fn half_alpha_over_transparent_recovers_the_source_color() {
    let mut buf = [0u8; 16];
    let mut s = Surface::new(&mut buf, dims(2, 2)).expect("surface");
    s.composite(0, 0, [200, 100, 50, 128]);
    assert_eq!(&buf[0..4], &[200, 100, 50, 128]);
}

#[test]
fn half_alpha_over_opaque_rounds_to_nearest() {
    let mut buf = [0u8; 16];
    let mut s = Surface::new(&mut buf, dims(2, 2)).expect("surface");
    s.composite(0, 0, [100, 100, 100, 255]);
    s.composite(0, 0, [0, 0, 0, 128]);
    assert_eq!(&buf[0..4], &[50, 50, 50, 255]);
}

#[test]
fn clear_zeroes_the_frame_region() {
    let mut buf = [5u8; 16];
    let mut s = Surface::new(&mut buf, dims(2, 2)).expect("surface");
    s.clear();
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn out_of_bounds_composite_is_ignored() {
    let mut buf = [0u8; 16];
    let mut s = Surface::new(&mut buf, dims(2, 2)).expect("surface");
    s.composite(-1, 0, [1, 1, 1, 255]);
    s.composite(2, 0, [1, 1, 1, 255]);
    s.composite(0, 2, [1, 1, 1, 255]);
    assert!(buf.iter().all(|&b| b == 0));
}

#[test]
fn spoil_paints_opaque_black_with_a_red_cross() {
    let mut buf = std::vec![0u8; 16 * 16 * 4];
    let mut s = Surface::new(&mut buf, dims(16, 16)).expect("surface");
    s.spoil();
    // Every pixel is opaque: a spoiled frame can never read as transparent.
    assert!(buf.chunks_exact(4).all(|px| px[3] == 255));
    // The top-left corner is on the main diagonal and painted red.
    assert_eq!(&buf[0..4], &[255, 0, 0, 255]);
    // A pixel off both diagonals stays black.
    let off = (10 * 16 + 2) * 4;
    assert_eq!(&buf[off..off + 4], &[0, 0, 0, 255]);
    // The cross is visibly present.
    let red = buf.chunks_exact(4).filter(|px| px[0] == 255).count();
    assert!(red >= 16);
}

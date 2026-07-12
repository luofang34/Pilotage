#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_scene::{MAX_STACK_DEPTH, Rgba8};

use super::*;
use crate::error::RasterError;
use crate::fixed::Fx;
use crate::surface::PixelRect;

fn full() -> PixelRect {
    PixelRect {
        left: 0,
        top: 0,
        right: 100,
        bottom: 100,
    }
}

fn corners(x: f32, y: f32, w: f32, h: f32) -> [[Fx; 2]; 4] {
    let s = |a: f32, b: f32| [Fx::snap(a).expect("finite"), Fx::snap(b).expect("finite")];
    [s(x, y), s(x + w, y), s(x + w, y + h), s(x, y + h)]
}

#[test]
fn save_restore_round_trips_paints() {
    let mut st = RenderState::new(full());
    st.set_fill(Rgba8::rgb(1, 2, 3));
    st.save().expect("save");
    st.set_fill(Rgba8::rgb(9, 9, 9));
    assert_eq!(st.current().fill, [9, 9, 9, 255]);
    st.restore().expect("restore");
    assert_eq!(st.current().fill, [1, 2, 3, 255]);
}

#[test]
fn stack_overflows_exactly_at_the_budget() {
    let mut st = RenderState::new(full());
    for _ in 0..MAX_STACK_DEPTH {
        st.save().expect("within budget");
    }
    assert_eq!(
        st.save(),
        Err(RasterError::StackOverflow {
            limit: MAX_STACK_DEPTH
        })
    );
}

#[test]
fn restore_without_save_is_unbalanced() {
    let mut st = RenderState::new(full());
    assert_eq!(st.restore(), Err(RasterError::UnbalancedRestore));
}

#[test]
fn clip_rect_intersects_successive_rectangles() {
    let mut st = RenderState::new(full());
    st.clip_rect(&corners(10.0, 10.0, 20.0, 30.0));
    assert_eq!(
        st.current().clip,
        PixelRect {
            left: 10,
            top: 10,
            right: 30,
            bottom: 40,
        }
    );
    st.clip_rect(&corners(0.0, 0.0, 20.0, 20.0));
    assert_eq!(
        st.current().clip,
        PixelRect {
            left: 10,
            top: 10,
            right: 20,
            bottom: 20,
        }
    );
}

#[test]
fn negative_stroke_width_clamps_to_zero() {
    let mut st = RenderState::new(full());
    st.set_stroke(Rgba8::rgb(0, 0, 0), -4.0).expect("finite");
    assert_eq!(st.current().stroke_width, 0.0);
}

#[test]
fn non_finite_stroke_width_is_rejected() {
    let mut st = RenderState::new(full());
    assert_eq!(
        st.set_stroke(Rgba8::rgb(0, 0, 0), f32::NAN),
        Err(RasterError::NonFinite)
    );
}

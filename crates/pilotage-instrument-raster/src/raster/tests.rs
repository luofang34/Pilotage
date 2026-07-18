#![allow(clippy::expect_used, clippy::panic)]

use std::vec::Vec;

use pilotage_instrument_scene::{Anchor, LayerId, MAX_SCENE_BYTES, PaintMode, Rgba8, SceneWriter};

use crate::{FrameId, FramebufferDims, RasterError, RenderReport, RenderStatus, render};

mod attitude_behavior;
mod conformance;
mod frame_hashes;
mod readout_bounds;
mod work_budget;

const BLACK: Rgba8 = Rgba8::rgb(0, 0, 0);
const WHITE: Rgba8 = Rgba8::rgb(255, 255, 255);

/// Wraps `build` in one valid Attitude layer so the scene passes the layer
/// contract, isolating tests from the layered-scene rules.
fn scene(build: impl FnOnce(&mut SceneWriter<'_>)) -> Vec<u8> {
    let mut buf = std::vec![0u8; MAX_SCENE_BYTES];
    let mut w = SceneWriter::new(&mut buf).expect("writer");
    w.begin_layer(LayerId::Attitude).expect("begin layer");
    build(&mut w);
    w.end_layer(LayerId::Attitude).expect("end layer");
    let n = w.finish();
    buf.truncate(n);
    buf
}

fn render_scene(bytes: &[u8], w: u32, h: u32) -> (Result<RenderReport, RasterError>, Vec<u8>) {
    let mut fb = std::vec![0u8; (w * h * 4) as usize];
    let res = render(
        bytes,
        &mut fb,
        FramebufferDims::tight(w, h),
        FrameId::default(),
    );
    (res, fb)
}

fn is_black_opaque(px: &[u8]) -> bool {
    px[0] == 0 && px[1] == 0 && px[2] == 0 && px[3] == 255
}

fn assert_spoiled(fb: &[u8]) {
    assert!(
        fb.chunks_exact(4).all(|px| px[3] == 255),
        "a spoiled frame is fully opaque"
    );
    assert!(
        fb[0] == 255 && fb[1] == 0 && fb[2] == 0 && fb[3] == 255,
        "the spoil cross reaches the top-left corner"
    );
    assert!(
        fb.chunks_exact(4).any(is_black_opaque),
        "the spoil pattern has a black field"
    );
}

fn painted_any(fb: &[u8]) -> bool {
    fb.chunks_exact(4).any(|px| px[3] != 0)
}

#[test]
fn renders_every_primitive_in_one_frame() {
    let bytes = scene(|w| {
        w.fill_color(WHITE).expect("fill");
        w.rect(PaintMode::Fill, 2.0, 2.0, 20.0, 20.0).expect("rect");
        w.stroke(Rgba8::rgb(200, 0, 0), 2.0).expect("stroke");
        w.line(0.0, 0.0, 30.0, 30.0).expect("line");
        w.polyline(&[[5.0, 40.0], [15.0, 60.0], [25.0, 40.0]])
            .expect("polyline");
        w.polygon(
            PaintMode::FillStroke,
            &[[40.0, 40.0], [60.0, 40.0], [50.0, 60.0]],
        )
        .expect("polygon");
        w.circle(PaintMode::Fill, 70.0, 30.0, 10.0)
            .expect("circle fill");
        w.circle(PaintMode::Stroke, 70.0, 70.0, 10.0)
            .expect("circle stroke");
        w.arc(30.0, 80.0, 12.0, 0.0, 3.0).expect("arc");
        w.text(50.0, 90.0, 10.0, Anchor::CENTER, "OK")
            .expect("text");
        w.save().expect("save");
        w.translate(80.0, 80.0).expect("translate");
        w.rotate(0.5).expect("rotate");
        w.clip_rect(0.0, 0.0, 15.0, 15.0).expect("clip");
        w.rect(PaintMode::Fill, 0.0, 0.0, 30.0, 30.0)
            .expect("clipped rect");
        w.restore().expect("restore");
    });
    let (res, fb) = render_scene(&bytes, 100, 100);
    let report = res.expect("renders");
    assert_eq!(report.status, RenderStatus::Painted);
    assert_eq!(report.scene_version, 1);
    assert_eq!(report.unknown_opcodes, 0);
    assert!(report.layers_present & (1 << LayerId::Attitude.to_u8()) != 0);
    assert!(painted_any(&fb));
}

#[test]
fn boundary_coordinates_render_without_error() {
    for (x, y, w, h) in [
        (0.0f32, 0.0f32, 10.0f32, 10.0f32), // origin corner
        (-5.0, -5.0, 12.0, 12.0),           // negative, partly off-screen
        (0.25, 0.25, 5.5, 5.5),             // subpixel offsets
        (30000.0, 0.0, 10.0, 10.0),         // large but in range, fully clipped
    ] {
        let bytes = scene(|writer| {
            writer.fill_color(WHITE).expect("fill");
            writer.rect(PaintMode::Fill, x, y, w, h).expect("rect");
        });
        let (res, _) = render_scene(&bytes, 50, 50);
        assert!(res.is_ok(), "coords ({x},{y},{w},{h}) should render");
    }
}

#[test]
fn out_of_range_coordinate_fails_and_spoils() {
    let bytes = scene(|w| {
        w.fill_color(WHITE).expect("fill");
        w.rect(PaintMode::Fill, 40000.0, 0.0, 10.0, 10.0)
            .expect("rect");
    });
    let (res, fb) = render_scene(&bytes, 32, 32);
    assert!(matches!(res, Err(RasterError::CoordinateOutOfRange { .. })));
    assert_spoiled(&fb);
}

type Emit = fn(&mut SceneWriter<'_>, f32);

/// One command builder per non-finite-sensitive slot; each places `bad` in a
/// different coordinate, transform, or size field.
fn non_finite_slots() -> Vec<Emit> {
    std::vec![
        |w, b| w.translate(b, 0.0).expect("enc"),
        |w, b| w.translate(0.0, b).expect("enc"),
        |w, b| w.rotate(b).expect("enc"),
        |w, b| w.stroke(BLACK, b).expect("enc"),
        |w, b| w.line(b, 0.0, 1.0, 1.0).expect("enc"),
        |w, b| w.line(0.0, b, 1.0, 1.0).expect("enc"),
        |w, b| w.line(0.0, 0.0, b, 1.0).expect("enc"),
        |w, b| w.line(0.0, 0.0, 1.0, b).expect("enc"),
        |w, b| w.polyline(&[[b, 0.0], [1.0, 1.0]]).expect("enc"),
        |w, b| w
            .polygon(PaintMode::Fill, &[[0.0, 0.0], [b, 1.0], [2.0, 2.0]])
            .expect("enc"),
        |w, b| w.rect(PaintMode::Fill, b, 0.0, 1.0, 1.0).expect("enc"),
        |w, b| w.rect(PaintMode::Fill, 0.0, b, 1.0, 1.0).expect("enc"),
        |w, b| w.rect(PaintMode::Fill, 0.0, 0.0, b, 1.0).expect("enc"),
        |w, b| w.rect(PaintMode::Fill, 0.0, 0.0, 1.0, b).expect("enc"),
        |w, b| w.circle(PaintMode::Fill, b, 0.0, 1.0).expect("enc"),
        |w, b| w.circle(PaintMode::Fill, 0.0, b, 1.0).expect("enc"),
        |w, b| w.circle(PaintMode::Fill, 0.0, 0.0, b).expect("enc"),
        |w, b| w.arc(b, 0.0, 1.0, 0.0, 1.0).expect("enc"),
        |w, b| w.arc(0.0, b, 1.0, 0.0, 1.0).expect("enc"),
        |w, b| w.arc(0.0, 0.0, b, 0.0, 1.0).expect("enc"),
        |w, b| w.arc(0.0, 0.0, 1.0, b, 1.0).expect("enc"),
        |w, b| w.arc(0.0, 0.0, 1.0, 0.0, b).expect("enc"),
        |w, b| w.text(b, 0.0, 10.0, Anchor::CENTER, "x").expect("enc"),
        |w, b| w.text(0.0, b, 10.0, Anchor::CENTER, "x").expect("enc"),
        |w, b| w.text(0.0, 0.0, b, Anchor::CENTER, "x").expect("enc"),
        |w, b| w.clip_rect(b, 0.0, 1.0, 1.0).expect("enc"),
        |w, b| w.clip_rect(0.0, b, 1.0, 1.0).expect("enc"),
        |w, b| w.clip_rect(0.0, 0.0, b, 1.0).expect("enc"),
        |w, b| w.clip_rect(0.0, 0.0, 1.0, b).expect("enc"),
    ]
}

#[test]
fn non_finite_in_any_slot_fails_and_spoils() {
    for (i, emit) in non_finite_slots().iter().enumerate() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let bytes = scene(|w| emit(w, bad));
            let (res, fb) = render_scene(&bytes, 32, 32);
            assert_eq!(res, Err(RasterError::NonFinite), "slot {i}, value {bad}");
            assert_spoiled(&fb);
        }
    }
}

#[test]
fn malformed_payload_fails_and_spoils() {
    // A LINE opcode (0x20) inside a valid layer but with a 4-byte payload
    // where 16 are required: the layer decoder rejects it.
    let mut bytes = std::vec![1u8]; // version
    push_cmd(&mut bytes, 0x50, &[LayerId::Attitude.to_u8()]); // begin layer
    push_cmd(&mut bytes, 0x01, &[]); // save (isolation)
    push_cmd(&mut bytes, 0x20, &[0, 0, 0, 0]); // malformed line
    push_cmd(&mut bytes, 0x02, &[]); // restore
    push_cmd(&mut bytes, 0x51, &[LayerId::Attitude.to_u8()]); // end layer
    let (res, fb) = render_scene(&bytes, 32, 32);
    assert!(matches!(res, Err(RasterError::Layer(_))));
    assert_spoiled(&fb);
}

fn push_cmd(bytes: &mut Vec<u8>, op: u8, payload: &[u8]) {
    bytes.push(op);
    bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    bytes.extend_from_slice(payload);
}

#[test]
fn unknown_opcode_is_counted_not_fatal() {
    let mut bytes = std::vec![1u8];
    push_cmd(&mut bytes, 0x50, &[LayerId::Attitude.to_u8()]);
    push_cmd(&mut bytes, 0x01, &[]);
    push_cmd(&mut bytes, 0x7f, &[9, 9]); // unknown opcode, skipped by length
    push_cmd(&mut bytes, 0x02, &[]);
    push_cmd(&mut bytes, 0x51, &[LayerId::Attitude.to_u8()]);
    let (res, _) = render_scene(&bytes, 16, 16);
    assert_eq!(res.expect("renders").unknown_opcodes, 1);
}

#[test]
fn zero_and_oversized_dimensions_are_rejected_before_painting() {
    let bytes = scene(|w| {
        w.fill_color(WHITE).expect("fill");
        w.rect(PaintMode::Fill, 0.0, 0.0, 4.0, 4.0).expect("rect");
    });
    let mut fb = std::vec![7u8; 64];
    assert_eq!(
        render(
            &bytes,
            &mut fb,
            FramebufferDims::tight(0, 4),
            FrameId::default()
        ),
        Err(RasterError::ZeroFramebuffer)
    );
    assert!(
        fb.iter().all(|&b| b == 7),
        "geometry error leaves the buffer"
    );
    let over = crate::MAX_DIMENSION + 1;
    assert!(matches!(
        render(
            &bytes,
            &mut fb,
            FramebufferDims::tight(over, 1),
            FrameId::default()
        ),
        Err(RasterError::FramebufferTooLarge { .. })
    ));
}

#[test]
fn undersized_buffer_is_rejected_before_painting() {
    let bytes = scene(|w| {
        w.rect(PaintMode::Fill, 0.0, 0.0, 4.0, 4.0).expect("rect");
    });
    let mut fb = std::vec![3u8; 10];
    assert!(matches!(
        render(
            &bytes,
            &mut fb,
            FramebufferDims::tight(8, 8),
            FrameId::default()
        ),
        Err(RasterError::FramebufferTooSmall { .. })
    ));
    assert!(fb.iter().all(|&b| b == 3));
}

#[test]
fn oversized_scene_fails_and_spoils() {
    let huge = std::vec![0u8; MAX_SCENE_BYTES + 1];
    let (res, fb) = render_scene(&huge, 32, 32);
    assert!(matches!(res, Err(RasterError::Layer(_))));
    assert_spoiled(&fb);
}

#[test]
fn maximum_stack_depth_renders() {
    let bytes = scene(|w| {
        // The layer isolation save is depth 1; nest 31 more to reach the
        // budget of 32 without overflowing.
        for _ in 0..(pilotage_instrument_scene::MAX_STACK_DEPTH - 1) {
            w.save().expect("save");
        }
        w.fill_color(WHITE).expect("fill");
        w.rect(PaintMode::Fill, 0.0, 0.0, 4.0, 4.0).expect("rect");
        for _ in 0..(pilotage_instrument_scene::MAX_STACK_DEPTH - 1) {
            w.restore().expect("restore");
        }
    });
    let (res, fb) = render_scene(&bytes, 16, 16);
    assert!(res.is_ok());
    assert!(painted_any(&fb));
}

#[test]
fn frame_identifiers_are_echoed() {
    let bytes = scene(|w| {
        w.fill_color(WHITE).expect("fill");
        w.rect(PaintMode::Fill, 0.0, 0.0, 4.0, 4.0).expect("rect");
    });
    let mut fb = std::vec![0u8; 16 * 16 * 4];
    let frame = FrameId {
        frame_generation: 7,
        render_generation: 42,
    };
    let report = render(&bytes, &mut fb, FramebufferDims::tight(16, 16), frame).expect("renders");
    assert_eq!(report.frame, frame);
}

#[test]
fn failure_overwrites_a_previously_plausible_frame() {
    // Pre-fill with a believable opaque frame, then fail: nothing of it
    // may survive.
    let mut fb = std::vec![0u8; 32 * 32 * 4];
    for px in fb.chunks_exact_mut(4) {
        px.copy_from_slice(&[20, 40, 60, 255]);
    }
    let bytes = scene(|w| {
        w.rect(PaintMode::Fill, 40000.0, 0.0, 10.0, 10.0)
            .expect("rect");
    });
    let res = render(
        &bytes,
        &mut fb,
        FramebufferDims::tight(32, 32),
        FrameId::default(),
    );
    assert!(res.is_err());
    assert_spoiled(&fb);
    assert!(
        !fb.chunks_exact(4).any(|px| px == [20, 40, 60, 255]),
        "no pixel of the stale frame remains"
    );
}

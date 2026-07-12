//! The individual conformance cases, grouped by concern. Each builder pushes
//! into the shared list so [`super::corpus`] keeps them in a stable order.

#![allow(clippy::expect_used, clippy::panic)]

use std::vec::Vec;

use pilotage_instrument_scene::{Anchor, LayerId, PaintMode, Rgba8};

use super::{CorpusEntry, Generator, f32le, gen_entry, in_layer, push_cmd, raw};

const WHITE: Rgba8 = Rgba8::rgb(255, 255, 255);
const RED: Rgba8 = Rgba8::rgb(200, 0, 0);
const AMBER: Rgba8 = Rgba8::rgb(255, 176, 0);

pub(super) fn valid_entries(out: &mut Vec<CorpusEntry>) {
    out.push(raw(
        "empty-background-canonical",
        "valid",
        Some("The byte-exact empty Background layer from the scene-layer protocol doc."),
        std::vec![1, 0x50, 1, 0, 0, 0x01, 0, 0, 0x02, 0, 0, 0x51, 1, 0, 0],
        true,
    ));
    out.push(raw(
        "attitude-every-drawing-opcode",
        "valid",
        Some("Every non-text drawing opcode in one Attitude layer."),
        in_layer(LayerId::Attitude, every_opcode),
        true,
    ));
    out.push(raw(
        "multi-layer-pfd",
        "valid",
        Some("Ascending Attitude, Tapes, Annunciation bands."),
        super::build_scene(multi_layer),
        true,
    ));
    out.push(raw(
        "transforms-nested-save",
        "valid",
        None,
        in_layer(LayerId::Attitude, |w| {
            w.save().expect("save");
            w.translate(120.0, 90.0).expect("translate");
            w.rotate(0.5).expect("rotate");
            w.line(-40.0, 0.0, 40.0, 0.0).expect("line");
            w.restore().expect("restore");
        }),
        true,
    ));
    out.push(raw(
        "clip-rect",
        "valid",
        None,
        in_layer(LayerId::Attitude, |w| {
            w.clip_rect(0.0, 0.0, 30.0, 30.0).expect("clip");
            w.fill_color(WHITE).expect("fill");
            w.rect(PaintMode::Fill, 0.0, 0.0, 60.0, 60.0).expect("rect");
        }),
        true,
    ));
    out.push(raw(
        "extreme-attitude",
        "extreme-attitude",
        Some("Large roll/translate placing the horizon far off center."),
        in_layer(LayerId::Attitude, |w| {
            w.save().expect("save");
            w.translate(240.0, 180.0).expect("translate");
            w.rotate(2.8).expect("rotate");
            w.line(-600.0, 0.0, 600.0, 0.0).expect("horizon");
            w.restore().expect("restore");
        }),
        true,
    ));
}

pub(super) fn symbology_entries(out: &mut Vec<CorpusEntry>) {
    out.push(raw(
        "extreme-coordinate-in-range",
        "extreme-coordinate",
        Some("Large but representable device coordinate; both backends draw it (fully clipped)."),
        in_layer(LayerId::Attitude, |w| {
            w.fill_color(WHITE).expect("fill");
            w.rect(PaintMode::Fill, 30000.0, 0.0, 10.0, 10.0)
                .expect("rect");
        }),
        true,
    ));
    out.push(raw(
        "background-imagery",
        "valid",
        Some("Replaceable Background band with fill and polygon."),
        in_layer(LayerId::Background, |w| {
            w.fill_color(Rgba8::rgb(20, 40, 60)).expect("fill");
            w.rect(PaintMode::Fill, 0.0, 0.0, 480.0, 180.0)
                .expect("sky");
            w.fill_color(Rgba8::rgb(60, 40, 20)).expect("fill");
            w.polygon(
                PaintMode::Fill,
                &[[0.0, 180.0], [480.0, 180.0], [480.0, 360.0], [0.0, 360.0]],
            )
            .expect("ground");
        }),
        true,
    ));
    out.push(raw(
        "guidance-cdi",
        "valid",
        None,
        in_layer(LayerId::Guidance, |w| {
            w.stroke(WHITE, 2.0).expect("stroke");
            w.line(240.0, 60.0, 240.0, 300.0).expect("course");
            w.line(200.0, 180.0, 280.0, 180.0).expect("deviation");
        }),
        true,
    ));
    out.push(raw(
        "failure-display",
        "failure-display",
        Some("Failure band reversion content; nothing may cover it."),
        in_layer(LayerId::Failure, |w| {
            w.fill_color(RED).expect("fill");
            w.rect(PaintMode::Fill, 0.0, 0.0, 480.0, 360.0)
                .expect("page");
        }),
        true,
    ));
    out.push(raw(
        "unknown-opcode-counted",
        "version-policy",
        Some("A well-framed unknown opcode inside a layer is counted, not fatal."),
        unknown_inside_layer(),
        true,
    ));
}

fn every_opcode(w: &mut pilotage_instrument_scene::SceneWriter<'_>) {
    w.fill_color(WHITE).expect("fill color");
    w.rect(PaintMode::Fill, 2.0, 2.0, 20.0, 20.0).expect("rect");
    w.stroke(RED, 2.0).expect("stroke");
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
    w.save().expect("save");
    w.translate(80.0, 80.0).expect("translate");
    w.rotate(0.5).expect("rotate");
    w.clip_rect(0.0, 0.0, 15.0, 15.0).expect("clip");
    w.rect(PaintMode::Fill, 0.0, 0.0, 30.0, 30.0)
        .expect("clipped rect");
    w.restore().expect("restore");
}

fn multi_layer(w: &mut pilotage_instrument_scene::SceneWriter<'_>) {
    w.begin_layer(LayerId::Attitude).expect("begin attitude");
    w.stroke(WHITE, 2.0).expect("stroke");
    w.line(0.0, 180.0, 480.0, 180.0).expect("horizon");
    w.end_layer(LayerId::Attitude).expect("end attitude");
    w.begin_layer(LayerId::Tapes).expect("begin tapes");
    w.fill_color(WHITE).expect("fill");
    w.rect(PaintMode::Stroke, 20.0, 120.0, 40.0, 120.0)
        .expect("speed tape");
    w.end_layer(LayerId::Tapes).expect("end tapes");
    w.begin_layer(LayerId::Annunciation)
        .expect("begin annunciation");
    w.fill_color(AMBER).expect("fill");
    w.rect(PaintMode::Fill, 210.0, 20.0, 60.0, 20.0)
        .expect("flag");
    w.end_layer(LayerId::Annunciation)
        .expect("end annunciation");
}

fn unknown_inside_layer() -> Vec<u8> {
    let mut b = std::vec![1u8];
    push_cmd(&mut b, 0x50, &[LayerId::Attitude.to_u8()]);
    push_cmd(&mut b, 0x01, &[]);
    push_cmd(&mut b, 0x7f, &[9, 9]);
    push_cmd(&mut b, 0x02, &[]);
    push_cmd(&mut b, 0x51, &[LayerId::Attitude.to_u8()]);
    b
}

pub(super) fn text_entries(out: &mut Vec<CorpusEntry>) {
    out.push(raw(
        "text-covered",
        "text",
        Some("Digit covered by the controlled pack; both backends render from the atlas."),
        in_layer(LayerId::Attitude, |w| {
            w.fill_color(WHITE).expect("fill");
            w.text(40.0, 40.0, 14.0, Anchor::BASELINE_LEFT, "1")
                .expect("text");
        }),
        true,
    ));
    out.push(raw(
        "text-uncovered",
        "text",
        Some("Character absent from the pack: both backends fail the run, neither substitutes a font."),
        in_layer(LayerId::Attitude, |w| {
            w.fill_color(WHITE).expect("fill");
            w.text(40.0, 40.0, 14.0, Anchor::BASELINE_LEFT, "#").expect("text");
        }),
        true,
    ));
}

pub(super) fn paint_fault_entries(out: &mut Vec<CorpusEntry>) {
    out.push(raw(
        "paint-non-finite",
        "paint-fault",
        Some("Gate accepts; the software rasterizer spoils on a non-finite coordinate and the browser interpreter's raw-argument guard throws before Canvas2D sees it."),
        in_layer(LayerId::Attitude, |w| {
            w.fill_color(WHITE).expect("fill");
            w.rect(PaintMode::Fill, f32::NAN, 0.0, 10.0, 10.0).expect("rect");
        }),
        true,
    ));
    out.push(raw(
        "paint-out-of-range",
        "paint-fault",
        Some("Gate accepts; the software rasterizer spoils on an out-of-range device coordinate and the interpreter's raw-argument guard rejects the same value."),
        in_layer(LayerId::Attitude, |w| {
            w.fill_color(WHITE).expect("fill");
            w.rect(PaintMode::Fill, 40000.0, 0.0, 10.0, 10.0).expect("rect");
        }),
        true,
    ));
    out.push(raw(
        "paint-too-many-vertices",
        "paint-fault",
        Some("Gate accepts; both backends enforce the shared per-shape vertex budget — the rasterizer via its fixed buffer, the interpreter via its path guard."),
        in_layer(LayerId::Attitude, |w| {
            w.stroke(WHITE, 1.0).expect("stroke");
            let pts: Vec<[f32; 2]> = (0..513).map(|i| [i as f32, (i % 7) as f32]).collect();
            w.polyline(&pts).expect("polyline");
        }),
        false,
    ));
    out.push(raw(
        "paint-non-finite-rotate",
        "paint-fault",
        Some("A NaN rotation poisons the transform: the rasterizer spoils on the first mapped point, the interpreter's angle guard throws at the rotate itself."),
        in_layer(LayerId::Attitude, |w| {
            w.rotate(f32::NAN).expect("rotate");
            w.fill_color(WHITE).expect("fill");
            w.rect(PaintMode::Fill, 10.0, 10.0, 5.0, 5.0).expect("rect");
        }),
        true,
    ));
    out.push(raw(
        "paint-out-of-range-translate",
        "paint-fault",
        Some("An out-of-range translation: the rasterizer rejects in device space, the interpreter rejects the raw argument."),
        in_layer(LayerId::Attitude, |w| {
            w.translate(40000.0, 0.0).expect("translate");
            w.fill_color(WHITE).expect("fill");
            w.rect(PaintMode::Fill, 0.0, 0.0, 5.0, 5.0).expect("rect");
        }),
        true,
    ));
    out.push(raw(
        "paint-non-finite-arc-angle",
        "paint-fault",
        Some("A non-finite arc start angle: the rasterizer spoils computing the sweep, the interpreter's angle guard throws."),
        in_layer(LayerId::Attitude, |w| {
            w.stroke(WHITE, 1.0).expect("stroke");
            w.arc(30.0, 30.0, 10.0, f32::NAN, 1.0).expect("arc");
        }),
        true,
    ));
    out.push(raw(
        "paint-out-of-range-stroke-width",
        "paint-fault",
        Some("A stroke width beyond the coordinate limit: the interpreter rejects the raw argument; the reference outcome pins whatever the rasterizer does with it."),
        in_layer(LayerId::Attitude, |w| {
            w.stroke(WHITE, 40000.0).expect("stroke");
            w.line(10.0, 10.0, 20.0, 20.0).expect("line");
        }),
        true,
    ));
}

pub(super) fn malformed_entries(out: &mut Vec<CorpusEntry>) {
    out.push(raw(
        "bad-version",
        "malformed",
        Some("Unsupported format version: both backends reject at framing."),
        std::vec![9, 0x01, 0, 0],
        false,
    ));
    let mut truncated = in_layer(LayerId::Attitude, every_opcode);
    truncated.pop();
    out.push(raw(
        "truncated-tail",
        "malformed",
        Some("A valid scene missing its last byte: framing and decode both reject."),
        truncated,
        false,
    ));
    out.push(raw(
        "malformed-line-payload",
        "malformed",
        Some("A LINE opcode with a 4-byte payload where 16 are required: JS framing passes on the declared length, the reference decoder rejects the payload."),
        malformed_line(),
        false,
    ));
    out.push(raw(
        "unknown-layer-id",
        "malformed",
        Some("A begin-layer marker naming an id this revision cannot place: framing passes, decode rejects."),
        std::vec![1, 0x50, 1, 0, 6],
        false,
    ));
    out.push(truncation_sweep());
}

fn truncation_sweep() -> CorpusEntry {
    CorpusEntry {
        name: "truncation-sweep",
        category: "truncation-sweep",
        notes: Some(
            "Framing verdict at every prefix length must match the reference command boundaries.",
        ),
        bytes: in_layer(LayerId::Attitude, |w| {
            w.fill_color(WHITE).expect("fill");
            w.rect(PaintMode::Fill, 0.0, 0.0, 10.0, 10.0).expect("rect");
            w.line(0.0, 0.0, 5.0, 5.0).expect("line");
        }),
        generator: None,
        trace: false,
        sweep: true,
    }
}

fn malformed_line() -> Vec<u8> {
    let mut b = std::vec![1u8];
    push_cmd(&mut b, 0x50, &[LayerId::Attitude.to_u8()]);
    push_cmd(&mut b, 0x01, &[]);
    push_cmd(&mut b, 0x20, &[0, 0, 0, 0]);
    push_cmd(&mut b, 0x02, &[]);
    push_cmd(&mut b, 0x51, &[LayerId::Attitude.to_u8()]);
    b
}

/// Builds a scene from a compact token DSL: `s` save, `r` restore, `x`
/// fill-rect body, `b<id>`/`e<id>` begin/end layer. Whitespace separates
/// tokens. This keeps each layer-structure case on one line.
fn layer_scene(spec: &str) -> Vec<u8> {
    let mut b = std::vec![1u8];
    for tok in spec.split_whitespace() {
        match tok.as_bytes() {
            b"s" => push_cmd(&mut b, 0x01, &[]),
            b"r" => push_cmd(&mut b, 0x02, &[]),
            b"x" => push_rect(&mut b),
            [b'b', id] => push_cmd(&mut b, 0x50, &[id - b'0']),
            [b'e', id] => push_cmd(&mut b, 0x51, &[id - b'0']),
            _ => panic!("bad layer token {tok}"),
        }
    }
    b
}

fn push_rect(b: &mut Vec<u8>) {
    let mut rect = std::vec![PaintMode::Fill.to_u8()];
    for v in [0.0f32, 0.0, 4.0, 4.0] {
        rect.extend_from_slice(&f32le(v));
    }
    push_cmd(b, 0x23, &rect);
}

fn layer_case(name: &'static str, notes: &'static str, spec: &str) -> CorpusEntry {
    raw(
        name,
        "layer-structure",
        Some(notes),
        layer_scene(spec),
        true,
    )
}

pub(super) fn layer_entries(out: &mut Vec<CorpusEntry>) {
    out.push(layer_case(
        "duplicate-layer",
        "Attitude opened twice.",
        "b1 s r e1 b1 s r e1",
    ));
    out.push(layer_case(
        "out-of-order-layer",
        "Tapes then Attitude descends the z-order.",
        "b2 s r e2 b1 s r e1",
    ));
    out.push(layer_case(
        "nested-layer",
        "Tapes opened while Attitude is open.",
        "b1 s b2 s r e2 r e1",
    ));
    out.push(layer_case("end-without-begin", "A stray end marker.", "e1"));
    out.push(layer_case(
        "end-mismatch",
        "Attitude closed by a Tapes end marker.",
        "b1 s r e2",
    ));
    out.push(layer_case(
        "unclosed-layer",
        "The scene ends with Attitude still open.",
        "b1 s r",
    ));
    out.push(layer_case(
        "unisolated-state",
        "A command before the isolation save.",
        "b1 x s r e1",
    ));
    out.push(layer_case(
        "unbalanced-state",
        "The isolation save is left open at layer end.",
        "b1 s e1",
    ));
    out.push(layer_case(
        "command-outside-layer",
        "A drawing command before any layer opens.",
        "x",
    ));
    out.push(raw(
        "unknown-opcode-outside-layer",
        "layer-structure",
        Some("An unknown opcode decodes but sits outside any layer."),
        std::vec![1, 0x7f, 0, 0],
        true,
    ));
}

pub(super) fn budget_entries(out: &mut Vec<CorpusEntry>) {
    let a = LayerId::Attitude.to_u8();
    out.push(gen_entry(
        "stack-depth-at-limit",
        "budget",
        Some("Peak graphics-state depth equals the 32 budget."),
        Generator::NestSaves {
            layer: a,
            extra_saves: 31,
        },
    ));
    out.push(gen_entry(
        "stack-depth-over-limit",
        "budget",
        Some("One save past the depth budget."),
        Generator::NestSaves {
            layer: a,
            extra_saves: 32,
        },
    ));
    out.push(gen_entry(
        "layer-commands-at-limit",
        "budget",
        Some("Layer command count equals the 4096 budget."),
        Generator::RepeatUnknown {
            layer: a,
            count: 4094,
        },
    ));
    out.push(gen_entry(
        "layer-commands-over-limit",
        "budget",
        Some("One command past the per-layer budget."),
        Generator::RepeatUnknown {
            layer: a,
            count: 4095,
        },
    ));
    out.push(gen_entry(
        "scene-bytes-at-limit",
        "budget",
        Some("Encoded scene equals the 65536-byte budget."),
        Generator::FillBytes {
            layer: a,
            total_len: 65536,
        },
    ));
    out.push(gen_entry(
        "scene-bytes-over-limit",
        "budget",
        Some("One byte past the scene budget."),
        Generator::FillBytes {
            layer: a,
            total_len: 65537,
        },
    ));
}

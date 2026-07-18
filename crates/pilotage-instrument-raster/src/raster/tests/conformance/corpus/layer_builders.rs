//! Layer-discipline corpus cases: a compact token DSL builds each
//! begin/end/save/restore structure on one line, covering the band
//! ordering and state-isolation-envelope rules both backends enforce.

#![allow(clippy::expect_used, clippy::panic)]

use std::vec::Vec;

use pilotage_instrument_scene::{LayerId, PaintMode};

use super::{CorpusEntry, Generator, f32le, gen_entry, push_cmd, raw};

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

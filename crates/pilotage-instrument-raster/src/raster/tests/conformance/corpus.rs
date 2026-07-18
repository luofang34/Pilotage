//! The reviewed conformance corpus: one deterministic byte stream per
//! semantic case, authored once here and pinned into the shared golden that
//! both the reference rasterizer and the browser Canvas interpreter replay.
//!
//! This core file owns the entry type, the byte-level builders, and the
//! reconstructable budget-boundary [`Generator`]; the individual cases live in
//! [`builders`]. Budget streams that would be megabytes of hex are carried as a
//! generator descriptor both backends reconstruct identically, so the golden
//! stays small and the corpus hash still covers them.

#![allow(clippy::expect_used, clippy::panic)]

mod builders;
mod layer_builders;

use std::vec::Vec;

use pilotage_instrument_scene::{LayerId, MAX_SCENE_BYTES, PaintMode, SceneWriter};

/// A reconstructable budget-boundary stream, so the golden need not carry a
/// 64 KiB scene as hex. Both backends build byte-identical output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Generator {
    /// A single layer padded by one unknown-opcode command so the whole scene
    /// is exactly `total_len` bytes.
    FillBytes { layer: u8, total_len: usize },
    /// A single layer holding `count` empty unknown-opcode commands between the
    /// isolation save/restore (layer command count is `count + 2`).
    RepeatUnknown { layer: u8, count: usize },
    /// A single layer nesting `extra_saves` saves above the isolation save
    /// (peak graphics-state depth is `extra_saves + 1`).
    NestSaves { layer: u8, extra_saves: usize },
}

impl Generator {
    /// Descriptor tag used in the golden and mirrored by the browser builder.
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::FillBytes { .. } => "fill_bytes",
            Self::RepeatUnknown { .. } => "repeat_unknown",
            Self::NestSaves { .. } => "nest_saves",
        }
    }

    pub(super) fn layer(&self) -> u8 {
        match *self {
            Self::FillBytes { layer, .. }
            | Self::RepeatUnknown { layer, .. }
            | Self::NestSaves { layer, .. } => layer,
        }
    }

    /// The single numeric parameter carried alongside `kind`/`layer`.
    pub(super) fn param(&self) -> usize {
        match *self {
            Self::FillBytes { total_len, .. } => total_len,
            Self::RepeatUnknown { count, .. } => count,
            Self::NestSaves { extra_saves, .. } => extra_saves,
        }
    }

    fn build(&self) -> Vec<u8> {
        match *self {
            Self::FillBytes { layer, total_len } => fill_bytes(layer, total_len),
            Self::RepeatUnknown { layer, count } => repeat_unknown(layer, count),
            Self::NestSaves { layer, extra_saves } => nest_saves(layer, extra_saves),
        }
    }
}

/// One corpus case: its identity, the bytes to replay, and flags steering how
/// much of the semantic outcome the golden pins.
pub(super) struct CorpusEntry {
    pub(super) name: &'static str,
    pub(super) category: &'static str,
    pub(super) notes: Option<&'static str>,
    pub(super) bytes: Vec<u8>,
    pub(super) generator: Option<Generator>,
    /// Emit the decoded command trace (and, for accepted non-text cases, the
    /// predicted Canvas method trace). Off for oversized budget streams.
    pub(super) trace: bool,
    /// This case drives the truncation sweep rather than a single verdict.
    pub(super) sweep: bool,
}

pub(super) fn f32le(v: f32) -> [u8; 4] {
    v.to_le_bytes()
}

pub(super) fn push_cmd(bytes: &mut Vec<u8>, op: u8, payload: &[u8]) {
    bytes.push(op);
    let len = u16::try_from(payload.len()).expect("payload fits u16");
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(payload);
}

pub(super) fn build_scene(f: impl FnOnce(&mut SceneWriter<'_>)) -> Vec<u8> {
    let mut buf = std::vec![0u8; MAX_SCENE_BYTES];
    let mut w = SceneWriter::new(&mut buf).expect("writer");
    f(&mut w);
    let n = w.finish();
    buf.truncate(n);
    buf
}

pub(super) fn in_layer(id: LayerId, f: impl FnOnce(&mut SceneWriter<'_>)) -> Vec<u8> {
    build_scene(|w| {
        w.begin_layer(id).expect("begin");
        f(w);
        w.end_layer(id).expect("end");
    })
}

pub(super) fn raw(
    name: &'static str,
    category: &'static str,
    notes: Option<&'static str>,
    bytes: Vec<u8>,
    trace: bool,
) -> CorpusEntry {
    CorpusEntry {
        name,
        category,
        notes,
        bytes,
        generator: None,
        trace,
        sweep: false,
    }
}

pub(super) fn gen_entry(
    name: &'static str,
    category: &'static str,
    notes: Option<&'static str>,
    g: Generator,
) -> CorpusEntry {
    CorpusEntry {
        name,
        category,
        notes,
        bytes: g.build(),
        generator: Some(g),
        trace: false,
        sweep: false,
    }
}

fn fill_bytes(layer: u8, total_len: usize) -> Vec<u8> {
    let mut b = std::vec![1u8];
    push_cmd(&mut b, 0x50, &[layer]);
    push_cmd(&mut b, 0x01, &[]);
    push_cmd(&mut b, 0x7f, &std::vec![0u8; total_len - 18]);
    push_cmd(&mut b, 0x02, &[]);
    push_cmd(&mut b, 0x51, &[layer]);
    b
}

fn repeat_unknown(layer: u8, count: usize) -> Vec<u8> {
    let mut b = std::vec![1u8];
    push_cmd(&mut b, 0x50, &[layer]);
    push_cmd(&mut b, 0x01, &[]);
    for _ in 0..count {
        push_cmd(&mut b, 0x7f, &[]);
    }
    push_cmd(&mut b, 0x02, &[]);
    push_cmd(&mut b, 0x51, &[layer]);
    b
}

fn nest_saves(layer: u8, extra_saves: usize) -> Vec<u8> {
    let mut b = std::vec![1u8];
    push_cmd(&mut b, 0x50, &[layer]);
    push_cmd(&mut b, 0x01, &[]);
    for _ in 0..extra_saves {
        push_cmd(&mut b, 0x01, &[]);
    }
    let mut rect = std::vec![PaintMode::Fill.to_u8()];
    for v in [0.0f32, 0.0, 1.0, 1.0] {
        rect.extend_from_slice(&f32le(v));
    }
    push_cmd(&mut b, 0x23, &rect);
    for _ in 0..extra_saves {
        push_cmd(&mut b, 0x02, &[]);
    }
    push_cmd(&mut b, 0x02, &[]);
    push_cmd(&mut b, 0x51, &[layer]);
    b
}

/// The full reviewed corpus, in a stable order.
pub(super) fn corpus() -> Vec<CorpusEntry> {
    let mut out = Vec::new();
    builders::valid_entries(&mut out);
    builders::symbology_entries(&mut out);
    builders::text_entries(&mut out);
    builders::paint_fault_entries(&mut out);
    builders::malformed_entries(&mut out);
    layer_builders::layer_entries(&mut out);
    layer_builders::budget_entries(&mut out);
    out
}

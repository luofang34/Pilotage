//! Versioned draw-command IR for instrument panels (ADR-0017).
//!
//! Instrument components emit an ordered list of abstract 2D drawing
//! commands instead of touching any graphics API; backends (browser
//! Canvas2D, wgpu, software raster, embedded framebuffer) interpret the
//! same encoding. The vocabulary is deliberately small and versioned:
//! growth is by appending opcodes, never redefining them, and decoders
//! treat unknown opcodes as counted skips so an older backend degrades
//! gracefully against a newer core.
//!
//! The crate is `no_std` and allocation-free: [`SceneWriter`] encodes into
//! a caller-provided byte buffer and reports overflow as an error;
//! [`SceneCmds`] decodes by borrowing from the encoded bytes.

#![no_std]

#[cfg(test)]
extern crate std;

mod cmd;
mod color;
mod decode;
mod encode;
mod layer;
mod opcode;
mod text_metrics;

pub use cmd::{Anchor, Cmd, HAlign, MAX_TEXT_BYTES, PaintMode, PointsRef, VAlign};
pub use color::Rgba8;
pub use decode::{DecodeError, SceneCmds};
pub use encode::{SceneError, SceneWriter};
pub use layer::{
    LAYER_COUNT, LayerError, LayerId, LayerReport, MAX_LAYER_COMMANDS, MAX_SCENE_BYTES,
    MAX_STACK_DEPTH, validate_layers,
};
pub use text_metrics::{
    NOMINAL_ADVANCE_RATIO, NOMINAL_GLYPH_RATIO, nominal_text_ink_width, nominal_text_width,
};

/// Format version written as the first byte of every encoded scene.
pub const SCENE_FORMAT_VERSION: u8 = 1;

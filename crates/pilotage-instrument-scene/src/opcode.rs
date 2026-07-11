//! Wire opcodes shared by the encoder and decoder.
//!
//! Opcode space is append-only: values are never reused or redefined
//! (ADR-0017). 0x50–0x51 are the layer markers (REN-01); 0x52–0x5F stay
//! reserved for the layer vocabulary.

pub(crate) const SAVE: u8 = 0x01;
pub(crate) const RESTORE: u8 = 0x02;
pub(crate) const TRANSLATE: u8 = 0x03;
pub(crate) const ROTATE: u8 = 0x04;
pub(crate) const FILL_COLOR: u8 = 0x10;
pub(crate) const STROKE: u8 = 0x11;
pub(crate) const LINE: u8 = 0x20;
pub(crate) const POLYLINE: u8 = 0x21;
pub(crate) const POLYGON: u8 = 0x22;
pub(crate) const RECT: u8 = 0x23;
pub(crate) const CIRCLE: u8 = 0x24;
pub(crate) const ARC: u8 = 0x25;
pub(crate) const TEXT: u8 = 0x30;
pub(crate) const CLIP_RECT: u8 = 0x40;
pub(crate) const BEGIN_LAYER: u8 = 0x50;
pub(crate) const END_LAYER: u8 = 0x51;

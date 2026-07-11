//! The frozen scene-layer and safety-compositor contract.
//!
//! A *layered scene* partitions its commands into bounded, named,
//! z-ordered criticality bands so a compositor can guarantee that
//! optional background imagery never covers, suppresses, or prevents
//! primary flight information, warnings, or failure indications
//! (AC 25-11B's mixed-criticality display concern, applied to this IR).
//!
//! Contract rules, enforced by [`validate_layers`]:
//!
//! - Each present layer appears at most once, opened and closed by
//!   matching [`Cmd::BeginLayer`]/[`Cmd::EndLayer`] markers, in strictly
//!   ascending [`LayerId`] order. Encoding order *is* z-order: painters
//!   run the commands front to back exactly as encoded, so a validated
//!   scene cannot paint background over a critical band.
//! - Layers never nest, and every drawing command sits inside exactly
//!   one layer.
//! - Every layer contains one mandatory outer save/restore envelope.
//!   Commands outside that envelope, a restore below it, or a save left
//!   open at layer end fail the frame. The envelope covers transform,
//!   clip, and paint state, so no lower band can affect a higher band.
//! - Frames are bounded: at most [`MAX_LAYER_COMMANDS`] commands per
//!   layer, [`MAX_STACK_DEPTH`] saved graphics states, and
//!   [`MAX_SCENE_BYTES`] encoded bytes per scene.
//! - Unknown *opcodes* inside a layer are counted skips (version
//!   policy, ADR-0017); an unknown *layer id* fails the whole frame at
//!   decode, because content whose criticality cannot be placed must
//!   not be painted. Extending [`LayerId`] therefore requires a scene
//!   format version bump, unlike ordinary appended opcodes.
//! - One frame is one encoded scene; frame generation/identity is the
//!   transport's concern (e.g. the WASM render generation), not encoded
//!   per layer. Each layer is owned by exactly one producer per frame —
//!   the duplicate rule makes split ownership structurally impossible.
//!
//! The SVS/raster boundary: backend-owned raster or depth imagery (such
//! as an SVS terrain layer) composes strictly *below* [`LayerId::Attitude`],
//! in the band [`LayerId::Background`] occupies. A scene rendered with no
//! background layer at all must still contain its complete critical
//! overlay — panels guarantee that, and the invariance is pinned by
//! tests in `pilotage-instrument-panels`.

use crate::cmd::Cmd;
use crate::decode::{DecodeError, SceneCmds};

/// Number of defined layers.
pub const LAYER_COUNT: usize = 6;

/// Most commands one layer may carry.
pub const MAX_LAYER_COMMANDS: usize = 4096;

/// Deepest graphics-state stack accepted inside one layer.
///
/// The mandatory layer-isolation save occupies depth one. Panel drawing
/// code may therefore nest at most 31 additional saves.
pub const MAX_STACK_DEPTH: usize = 32;

/// Largest encoded scene a conforming backend must accept.
pub const MAX_SCENE_BYTES: usize = 64 * 1024;

/// Z-ordered, criticality-banded scene layers.
///
/// The discriminant is both the wire encoding and the z-order: greater
/// values paint later (above) and carry higher display criticality.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayerId {
    /// Optional imagery only: flat attitude shading or an SVS raster.
    /// The single band a compositor may replace or drop.
    Background = 0,
    /// Primary attitude symbology: pitch ladder frame, roll scale,
    /// aircraft reference.
    Attitude = 1,
    /// Tapes and readouts: speed/altitude/VSI tapes, data boxes.
    Tapes = 2,
    /// Navigation guidance: CDI, deviation scales.
    Guidance = 3,
    /// Flags, miscompares, and failure annunciations over everything
    /// they annunciate.
    Annunciation = 4,
    /// Display-level failure/reversion content; nothing may cover it.
    Failure = 5,
}

impl LayerId {
    /// Wire encoding.
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decodes the wire byte; `None` for ids this revision cannot place.
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Background),
            1 => Some(Self::Attitude),
            2 => Some(Self::Tapes),
            3 => Some(Self::Guidance),
            4 => Some(Self::Annunciation),
            5 => Some(Self::Failure),
            _ => None,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Why a scene failed layer validation. Any of these fails the whole
/// frame before anything becomes visible.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum LayerError {
    /// The command stream itself is malformed (truncated, bad version,
    /// bad payload — including an unknown layer id).
    #[error("scene decoding failed: {0}")]
    Decode(#[from] DecodeError),
    /// A layer was opened twice in one frame.
    #[error("layer {layer:?} appears more than once")]
    DuplicateLayer {
        /// The layer opened again.
        layer: LayerId,
    },
    /// A layer was opened at or below the most recently opened layer's
    /// z-order.
    #[error("layer {layer:?} is not in strictly ascending order")]
    OutOfOrder {
        /// The layer opened out of order.
        layer: LayerId,
    },
    /// A layer was opened while another was still open.
    #[error("layer {layer:?} is nested inside another layer")]
    NestedLayer {
        /// The layer whose open marker nested.
        layer: LayerId,
    },
    /// An end marker appeared with no layer open.
    #[error("layer {layer:?} ends without a matching begin marker")]
    EndWithoutBegin {
        /// The layer the stray end marker named.
        layer: LayerId,
    },
    /// An end marker named a different layer than the open one.
    #[error("layer {end:?} ends while {open:?} is open")]
    EndMismatch {
        /// The layer currently open.
        open: LayerId,
        /// The layer the end marker named.
        end: LayerId,
    },
    /// The scene ended with a layer still open (typically truncation at
    /// a command boundary).
    #[error("layer {layer:?} is not closed")]
    UnclosedLayer {
        /// The layer left open.
        layer: LayerId,
    },
    /// A command appeared outside any layer.
    #[error("drawing command appears outside a layer")]
    CommandOutsideLayer,
    /// A command appeared before the isolation save or after its matching
    /// restore.
    #[error("layer {layer:?} has a command outside its state-isolation envelope")]
    UnisolatedState {
        /// The layer that did not isolate all commands.
        layer: LayerId,
    },
    /// A restore crossed the layer isolation boundary, or a save was left
    /// open at layer end.
    #[error("layer {layer:?} has unbalanced graphics state")]
    UnbalancedState {
        /// The layer whose save/restore pairing broke.
        layer: LayerId,
    },
    /// A layer exceeded [`MAX_STACK_DEPTH`].
    #[error("layer {layer:?} requests graphics-state depth {depth}")]
    StackOverCapacity {
        /// The over-budget layer.
        layer: LayerId,
        /// The depth the rejected save would create.
        depth: usize,
    },
    /// A layer exceeded [`MAX_LAYER_COMMANDS`].
    #[error("layer {layer:?} exceeds the command budget")]
    OverCapacity {
        /// The over-budget layer.
        layer: LayerId,
    },
    /// The encoded scene exceeds [`MAX_SCENE_BYTES`].
    #[error("scene uses {bytes} encoded bytes")]
    SceneTooLarge {
        /// The encoded byte length.
        bytes: usize,
    },
}

/// What a validated layered scene contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerReport {
    /// Bit `i` set means the layer with discriminant `i` is present.
    pub present: u8,
    /// Command count per layer (markers excluded).
    pub commands: [u16; LAYER_COUNT],
    /// Byte range of each present layer's contents, including its outer
    /// Save/Restore but excluding its markers, as offsets into the encoded
    /// scene. Equal ranges across two scenes identify equal layer content.
    pub ranges: [Option<(usize, usize)>; LAYER_COUNT],
    /// Unknown opcodes skipped inside layers (version policy).
    pub unknown: u32,
}

impl LayerReport {
    /// Whether `layer` is present.
    pub const fn contains(&self, layer: LayerId) -> bool {
        self.present & (1 << layer.index()) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IsolationState {
    AwaitingSave,
    Active,
    Closed,
}

struct OpenLayer {
    id: LayerId,
    content_start: usize,
    depth: usize,
    isolation: IsolationState,
}

impl OpenLayer {
    fn new(id: LayerId, content_start: usize) -> Self {
        Self {
            id,
            content_start,
            depth: 0,
            isolation: IsolationState::AwaitingSave,
        }
    }

    fn accept(&mut self, command: &Cmd<'_>) -> Result<(), LayerError> {
        match command {
            Cmd::Save => self.push_state(),
            Cmd::Restore => self.pop_state(),
            _ if self.isolation == IsolationState::Active => Ok(()),
            _ => Err(LayerError::UnisolatedState { layer: self.id }),
        }
    }

    fn push_state(&mut self) -> Result<(), LayerError> {
        if self.isolation == IsolationState::Closed {
            return Err(LayerError::UnisolatedState { layer: self.id });
        }
        let depth = self.depth.saturating_add(1);
        if depth > MAX_STACK_DEPTH {
            return Err(LayerError::StackOverCapacity {
                layer: self.id,
                depth,
            });
        }
        self.depth = depth;
        self.isolation = IsolationState::Active;
        Ok(())
    }

    fn pop_state(&mut self) -> Result<(), LayerError> {
        if self.isolation != IsolationState::Active || self.depth == 0 {
            return Err(LayerError::UnbalancedState { layer: self.id });
        }
        self.depth -= 1;
        if self.depth == 0 {
            self.isolation = IsolationState::Closed;
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), LayerError> {
        if self.isolation == IsolationState::Closed && self.depth == 0 {
            Ok(())
        } else {
            Err(LayerError::UnbalancedState { layer: self.id })
        }
    }
}

fn open_layer(
    report: &LayerReport,
    open: &mut Option<OpenLayer>,
    last_opened: &mut Option<LayerId>,
    layer: LayerId,
    content_start: usize,
) -> Result<(), LayerError> {
    if open.is_some() {
        return Err(LayerError::NestedLayer { layer });
    }
    if report.contains(layer) {
        return Err(LayerError::DuplicateLayer { layer });
    }
    if last_opened.is_some_and(|previous| layer <= previous) {
        return Err(LayerError::OutOfOrder { layer });
    }
    *last_opened = Some(layer);
    *open = Some(OpenLayer::new(layer, content_start));
    Ok(())
}

fn close_layer(
    report: &mut LayerReport,
    open: &mut Option<OpenLayer>,
    layer: LayerId,
    content_end: usize,
) -> Result<(), LayerError> {
    let Some(inside) = open.take() else {
        return Err(LayerError::EndWithoutBegin { layer });
    };
    if inside.id != layer {
        return Err(LayerError::EndMismatch {
            open: inside.id,
            end: layer,
        });
    }
    inside.finish()?;
    report.present |= 1 << layer.index();
    if let Some(range) = report.ranges.get_mut(layer.index()) {
        *range = Some((inside.content_start, content_end));
    }
    Ok(())
}

fn record_command(
    report: &mut LayerReport,
    inside: &mut OpenLayer,
    command: Cmd<'_>,
) -> Result<(), LayerError> {
    inside.accept(&command)?;
    let count = report
        .commands
        .get_mut(inside.id.index())
        .ok_or(LayerError::OverCapacity { layer: inside.id })?;
    if usize::from(*count) >= MAX_LAYER_COMMANDS {
        return Err(LayerError::OverCapacity { layer: inside.id });
    }
    *count = count.wrapping_add(1);
    if matches!(command, Cmd::Unknown { .. }) {
        report.unknown = report.unknown.wrapping_add(1);
    }
    Ok(())
}

/// Validates the layered-scene contract over an encoded scene and
/// reports what it contains. Structural corruption, ordering violations,
/// state leaks, and budget violations all fail the frame — a backend
/// must run this (or enforce the same rules) before anything becomes
/// visible.
pub fn validate_layers(scene: &[u8]) -> Result<LayerReport, LayerError> {
    if scene.len() > MAX_SCENE_BYTES {
        return Err(LayerError::SceneTooLarge { bytes: scene.len() });
    }
    let mut cmds = SceneCmds::new(scene)?;
    let mut report = LayerReport::default();
    let mut open: Option<OpenLayer> = None;
    let mut last_opened: Option<LayerId> = None;

    loop {
        let at = scene.len() - cmds.remaining();
        let Some(cmd) = cmds.next() else { break };
        match cmd? {
            Cmd::BeginLayer { layer } => {
                let content_start = scene.len() - cmds.remaining();
                open_layer(&report, &mut open, &mut last_opened, layer, content_start)?;
            }
            Cmd::EndLayer { layer } => {
                close_layer(&mut report, &mut open, layer, at)?;
            }
            command => {
                let Some(inside) = open.as_mut() else {
                    return Err(LayerError::CommandOutsideLayer);
                };
                record_command(&mut report, inside, command)?;
            }
        }
    }
    if let Some(inside) = open {
        return Err(LayerError::UnclosedLayer { layer: inside.id });
    }
    Ok(report)
}

#[cfg(test)]
mod tests;

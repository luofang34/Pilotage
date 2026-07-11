//! The decoded command vocabulary.

use crate::color::Rgba8;
use crate::layer::LayerId;

/// Longest text payload a single [`Cmd::Text`] may carry, in UTF-8 bytes.
pub const MAX_TEXT_BYTES: usize = 250;

/// Whether a shape is filled, stroked, or both, using the current paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintMode {
    /// Fill interior with the current fill color.
    Fill,
    /// Stroke outline with the current stroke color and width.
    Stroke,
    /// Fill, then stroke.
    FillStroke,
}

impl PaintMode {
    /// Wire encoding: bit 0 fill, bit 1 stroke.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Fill => 0b01,
            Self::Stroke => 0b10,
            Self::FillStroke => 0b11,
        }
    }

    /// Decodes the wire bits; `None` when neither bit is set.
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v & 0b11 {
            0b01 => Some(Self::Fill),
            0b10 => Some(Self::Stroke),
            0b11 => Some(Self::FillStroke),
            _ => None,
        }
    }
}

/// How text is positioned relative to its (x, y) point.
///
/// Wire encoding packs horizontal alignment in bits 0–1 and vertical in
/// bits 2–3. Text metrics are backend-owned (ADR-0017): anchors describe
/// intent; exact glyph geometry is the backend's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    /// Horizontal alignment of the text run against x.
    pub h: HAlign,
    /// Vertical alignment of the text run against y.
    pub v: VAlign,
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    /// x is the left edge.
    Left,
    /// x is the center.
    Center,
    /// x is the right edge.
    Right,
}

/// Vertical text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    /// y is the alphabetic baseline.
    Baseline,
    /// y is the vertical middle.
    Middle,
    /// y is the top edge.
    Top,
    /// y is the bottom edge.
    Bottom,
}

impl Anchor {
    /// Left-aligned on the baseline; the wire zero value.
    pub const BASELINE_LEFT: Self = Self {
        h: HAlign::Left,
        v: VAlign::Baseline,
    };

    /// Centered both ways; the workhorse for tape labels and readouts.
    pub const CENTER: Self = Self {
        h: HAlign::Center,
        v: VAlign::Middle,
    };

    /// Left-aligned, vertically centered.
    pub const MIDDLE_LEFT: Self = Self {
        h: HAlign::Left,
        v: VAlign::Middle,
    };

    /// Right-aligned, vertically centered.
    pub const MIDDLE_RIGHT: Self = Self {
        h: HAlign::Right,
        v: VAlign::Middle,
    };

    /// Wire encoding.
    pub const fn to_u8(self) -> u8 {
        let h = match self.h {
            HAlign::Left => 0,
            HAlign::Center => 1,
            HAlign::Right => 2,
        };
        let v = match self.v {
            VAlign::Baseline => 0,
            VAlign::Middle => 1,
            VAlign::Top => 2,
            VAlign::Bottom => 3,
        };
        h | (v << 2)
    }

    /// Decodes the wire byte; `None` for a reserved horizontal value.
    pub const fn from_u8(b: u8) -> Option<Self> {
        let h = match b & 0b11 {
            0 => HAlign::Left,
            1 => HAlign::Center,
            2 => HAlign::Right,
            _ => return None,
        };
        let v = match (b >> 2) & 0b11 {
            0 => VAlign::Baseline,
            1 => VAlign::Middle,
            2 => VAlign::Top,
            _ => VAlign::Bottom,
        };
        Some(Self { h, v })
    }
}

/// One decoded drawing command.
///
/// Coordinates are in the panel's logical space; transforms compose in
/// emission order with save/restore pairing exactly as in PostScript-style
/// 2D canvases.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd<'a> {
    /// Push a copy of the current transform + clip + paint state.
    Save,
    /// Pop to the most recently saved state.
    Restore,
    /// Translate the current transform by (x, y).
    Translate {
        /// X offset.
        x: f32,
        /// Y offset.
        y: f32,
    },
    /// Rotate the current transform; positive is clockwise in the
    /// y-down logical space.
    Rotate {
        /// Angle in radians.
        radians: f32,
    },
    /// Set the fill color for subsequent filled shapes.
    FillColor {
        /// The color.
        color: Rgba8,
    },
    /// Set the stroke color and width for subsequent stroked shapes.
    Stroke {
        /// The color.
        color: Rgba8,
        /// Line width in logical units.
        width: f32,
    },
    /// A straight line segment (always stroked).
    Line {
        /// Start x.
        x1: f32,
        /// Start y.
        y1: f32,
        /// End x.
        x2: f32,
        /// End y.
        y2: f32,
    },
    /// An open polyline through the points (always stroked).
    Polyline {
        /// Vertices as interleaved x, y pairs.
        points: PointsRef<'a>,
    },
    /// A closed polygon through the points.
    Polygon {
        /// Fill/stroke selection.
        mode: PaintMode,
        /// Vertices as interleaved x, y pairs.
        points: PointsRef<'a>,
    },
    /// An axis-aligned rectangle.
    Rect {
        /// Fill/stroke selection.
        mode: PaintMode,
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
    },
    /// A circle.
    Circle {
        /// Fill/stroke selection.
        mode: PaintMode,
        /// Center x.
        cx: f32,
        /// Center y.
        cy: f32,
        /// Radius.
        r: f32,
    },
    /// A circular arc (always stroked).
    Arc {
        /// Center x.
        cx: f32,
        /// Center y.
        cy: f32,
        /// Radius.
        r: f32,
        /// Start angle in radians; 0 is +x, positive sweeps clockwise in
        /// the y-down logical space.
        start: f32,
        /// Signed sweep in radians.
        sweep: f32,
    },
    /// A single run of text drawn with the current fill color.
    Text {
        /// Anchor point x.
        x: f32,
        /// Anchor point y.
        y: f32,
        /// Font size in logical units (cap height is backend-defined).
        size: f32,
        /// Positioning against (x, y).
        anchor: Anchor,
        /// The UTF-8 text.
        text: &'a str,
    },
    /// Intersect the current clip with an axis-aligned rectangle.
    ClipRect {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
    },
    /// Opens a z-ordered criticality layer (REN-01). Layered scenes obey
    /// the contract [`crate::validate_layers`] enforces: ascending,
    /// non-nested, non-repeating, every drawing command inside exactly
    /// one layer.
    BeginLayer {
        /// The layer being opened.
        layer: LayerId,
    },
    /// Closes the open layer; must name the layer that is open.
    EndLayer {
        /// The layer being closed.
        layer: LayerId,
    },
    /// A command from a newer format revision this decoder does not know;
    /// its payload was skipped. Consumers should count these, not fail.
    Unknown {
        /// The unrecognized opcode.
        opcode: u8,
    },
}

/// Borrowed little-endian vertex bytes: interleaved x, y `f32` pairs.
///
/// Kept as raw bytes so decoding never allocates; [`PointsRef::get`]
/// reads out one vertex at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointsRef<'a> {
    bytes: &'a [u8],
}

impl<'a> PointsRef<'a> {
    /// Wraps vertex bytes; `None` unless the length is a whole number of
    /// 8-byte (x, y) pairs.
    pub const fn from_bytes(bytes: &'a [u8]) -> Option<Self> {
        if bytes.len().is_multiple_of(8) {
            Some(Self { bytes })
        } else {
            None
        }
    }

    /// Number of vertices.
    pub const fn len(&self) -> usize {
        self.bytes.len() / 8
    }

    /// Whether there are no vertices.
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The i-th vertex, or `None` past the end.
    pub fn get(&self, i: usize) -> Option<[f32; 2]> {
        let at = i.checked_mul(8)?;
        let chunk = self.bytes.get(at..at.checked_add(8)?)?;
        let x = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let y = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        Some([x, y])
    }

    /// Iterator over vertices.
    pub fn iter(&self) -> impl Iterator<Item = [f32; 2]> + 'a {
        let bytes = self.bytes;
        (0..self.len()).filter_map(move |i| Self { bytes }.get(i))
    }
}

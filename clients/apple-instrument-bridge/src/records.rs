//! The plain-data records the FFI carries. Every field is a value the
//! runtime already publishes; these types only give it a name across
//! the boundary.

/// One panel descriptor, as plain data for the FFI consumer.
#[derive(uniffi::Record)]
pub struct BridgePanelDescriptor {
    /// The stable panel id.
    pub id: String,
    /// The operator-facing panel title.
    pub title: String,
    /// The required-layer bitset.
    pub required_layers: u32,
    /// The required state groups as a bitset over the wire tags.
    pub required_groups: u32,
    /// Width of the frame the runtime draws the panel at, in logical
    /// units.
    pub design_width: f32,
    /// Height of the frame the runtime draws the panel at, in logical
    /// units.
    pub design_height: f32,
    /// Background capability code: 0 not-used, 1 opaque, 2 cedeable.
    pub background_capability: u32,
}

/// One composition slot, as plain data for the FFI consumer. Slot
/// index is the paint order.
#[derive(uniffi::Record)]
pub struct BridgeCompositionSlot {
    /// The panel id this slot paints.
    pub panel: String,
    /// Left edge of the slot's rectangle, in screen units.
    pub x: f32,
    /// Top edge of the slot's rectangle, in screen units.
    pub y: f32,
    /// Width of the slot's rectangle, in screen units.
    pub width: f32,
    /// Height of the slot's rectangle, in screen units.
    pub height: f32,
}

/// The typed result of one render call: the producer status, the scene
/// bytes, the frame the scene was emitted at, and the panel's
/// successful-production generation.
#[derive(uniffi::Record)]
pub struct BridgeRenderOutcome {
    /// The typed producer status: the `RenderStatus` discriminant. Zero
    /// means the scene rendered and self-validated.
    pub status: u32,
    /// The typed scene bytes. Empty on every failure; the caller does
    /// not paint on a nonzero status.
    pub scene: Vec<u8>,
    /// Width of the frame the scene was emitted at. The caller
    /// compares it with the frame it prepared and does not paint on
    /// mismatch.
    pub frame_width: f32,
    /// Height of the frame the scene was emitted at.
    pub frame_height: f32,
    /// The panel's successful-production generation. It advances only
    /// on success, so it is a liveness signal no failed render can fake.
    pub generation: u32,
}

/// The typed result of one state-buffer write.
#[derive(Debug, PartialEq, Eq, uniffi::Record)]
pub struct BridgeWriteOutcome {
    /// Zero for acceptance. One means the frame exceeds capacity.
    pub status: u32,
    /// Length supplied by the caller.
    pub actual: u64,
    /// Maximum length accepted by the runtime.
    pub capacity: u64,
}

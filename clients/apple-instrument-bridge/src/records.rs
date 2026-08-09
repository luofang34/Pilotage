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

/// One panel result in a composition transaction.
#[derive(uniffi::Record)]
pub struct BridgeCompositionPanelOutcome {
    /// The panel index in the runtime registry.
    pub panel: u32,
    /// The composition transaction status.
    pub status: u32,
    /// The panel scene offset in the shared scene buffer.
    pub scene_offset: u32,
    /// The panel scene length in bytes.
    pub scene_len: u32,
    /// The width used to produce the panel scene.
    pub frame_width: f32,
    /// The height used to produce the panel scene.
    pub frame_height: f32,
    /// The panel generation after a successful transaction.
    pub generation: u32,
}

/// The typed result of one complete composition transaction.
#[derive(uniffi::Record)]
pub struct BridgeCompositionFrameOutcome {
    /// The transaction status. Zero means all panels committed.
    pub status: u32,
    /// All panel scenes in composition order. This is empty on failure.
    pub scene: Vec<u8>,
    /// The typed result for each composition slot.
    pub panels: Vec<BridgeCompositionPanelOutcome>,
    /// The composition generation after a successful transaction.
    pub generation: u32,
    /// The alert-step status from the same transaction.
    pub alert_status: u32,
    /// The number of active alerts.
    pub active_alert_count: u32,
    /// The independent alert path is faulted.
    pub alert_path_faulted: bool,
    /// The alert manager dropped events.
    pub alert_overflow: bool,
    /// The alert-manager generation.
    pub alert_manager_generation: u32,
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

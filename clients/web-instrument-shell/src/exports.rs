//! Explicit WASM resource ownership over the portable runtime.
//!
//! [`InstrumentRuntime`] is a JavaScript-owned wasm-bindgen resource.
//! Each resource wraps one [`Runtime`]; this module packs the typed
//! outcomes into the wire `u64` layouts and holds no logic of its own.

use pilotage_instrument_runtime::{AlertStepOutcome, RenderOutcome, RenderStatus, Runtime};
use wasm_bindgen::prelude::wasm_bindgen;

/// Packs a render outcome: status in bits 0..7, scene length in bits
/// 8..31, and generation in bits 32..63.
fn pack_render(outcome: RenderOutcome) -> u64 {
    let scene_len = u64::from(outcome.scene_len) & 0x00ff_ffff;
    (outcome.status as u64) | (scene_len << 8) | (u64::from(outcome.generation) << 32)
}

/// Packs an alert-step outcome: status in bits 0..7, active-alert count
/// in bits 8..15, faulted health in bit 16, overflow in bit 17, and the
/// manager generation in bits 32..63.
fn pack_alerts(outcome: AlertStepOutcome) -> u64 {
    (outcome.status as u64)
        | ((u64::from(outcome.active_count) & 0xff) << 8)
        | (u64::from(outcome.faulted) << 16)
        | (u64::from(outcome.overflow) << 17)
        | (u64::from(outcome.manager_generation) << 32)
}

/// The state-frame ABI version this module was built against.
#[wasm_bindgen]
pub fn abi_version() -> u32 {
    pilotage_instrument_runtime::abi_version()
}

/// One explicitly owned instrument renderer and its fixed-capacity buffers.
///
/// Construction does not allocate buffers. Call [`InstrumentRuntime::init`]
/// before querying pointers or rendering; calling it again replaces all
/// buffers, configuration, and generations and invalidates earlier pointers.
#[wasm_bindgen]
#[derive(Default)]
pub struct InstrumentRuntime {
    pub(crate) runtime: Option<Runtime>,
}

#[wasm_bindgen]
impl InstrumentRuntime {
    /// Creates an uninitialized resource with no buffers.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates or replaces this resource's runtime; returns 1 on success.
    pub fn init(&mut self) -> u32 {
        self.runtime = Some(Runtime::new());
        1
    }

    /// Linear-memory offset of the state-frame buffer, or zero before init.
    pub fn state_ptr(&self) -> u32 {
        self.runtime
            .as_ref()
            .map_or(0, |runtime| runtime.state().as_ptr() as u32)
    }

    /// Capacity of the state-frame buffer in bytes. The v7 frame is
    /// self-delimiting, so the writer needs a bound, not an exact size;
    /// growing the capacity is not a wire break.
    pub fn state_capacity(&self) -> u32 {
        Runtime::state_capacity() as u32
    }

    /// Linear-memory offset of the encoded-scene buffer, or zero before init.
    pub fn scene_ptr(&self) -> u32 {
        self.runtime
            .as_ref()
            .map_or(0, |runtime| runtime.scene().as_ptr() as u32)
    }

    /// Configures speed-tape bands in knots; pass all zeros to clear.
    /// Returns a stable [`RenderStatus`] code. This is sugar over the
    /// PFD's config blob: it splices the V_SPEEDS entry and leaves
    /// every other configured key alone.
    pub fn set_v_speeds(&mut self, vs0: f32, vs: f32, vfe: f32, vno: f32, vne: f32) -> u32 {
        self.runtime
            .as_mut()
            .map_or(RenderStatus::NotInitialized as u32, |runtime| {
                runtime.set_v_speeds(vs0, vs, vfe, vno, vne) as u32
            })
    }

    /// Replaces a panel's configuration with `blob` (the bounded
    /// key-TLV encoding), validated for framing and against the
    /// panel's declared schema before it is accepted — a refused blob
    /// leaves the previous configuration in effect. Returns a stable
    /// [`RenderStatus`] code.
    pub fn set_panel_config(&mut self, panel: u32, blob: &[u8]) -> u32 {
        self.runtime
            .as_mut()
            .map_or(RenderStatus::NotInitialized as u32, |runtime| {
                runtime.set_panel_config(panel, blob) as u32
            })
    }

    /// Wrapping count of decoded frames' group tags this build cannot
    /// place — a producer ahead of this consumer is visible, not
    /// silent (mirrors the scene path's unknownOpcodes).
    pub fn state_unknown_groups(&self) -> u32 {
        self.runtime.as_ref().map_or(0, Runtime::unknown_groups)
    }

    /// Wrapping count of known groups whose payloads carried grown
    /// tails this build skipped.
    pub fn state_extended_groups(&self) -> u32 {
        self.runtime.as_ref().map_or(0, Runtime::extended_groups)
    }

    /// Renders a panel and returns status in bits 0..7, scene length in bits
    /// 8..31, and successful generation in bits 32..63. Failure has a zero
    /// scene length and never advances generation.
    ///
    /// Successful scene bytes remain valid until this resource's next render
    /// or init call; failure leaves scratch bytes unspecified.
    pub fn render_result(&mut self, panel: u32) -> u64 {
        self.runtime.as_mut().map_or_else(
            || pack_render(RenderOutcome::failure(RenderStatus::NotInitialized, 0)),
            |runtime| pack_render(runtime.render(panel)),
        )
    }

    /// Steps the alert manager once against the current state block and
    /// caches the output every subsequent panel render consumes, so all
    /// panels in a frame share one semantic alert state. `now_ms` is the
    /// caller's monotonic clock (the manager never reads an interior
    /// clock); `path_healthy == 0` marks the independent display/alerting
    /// path monitor faulted, which flags the output untrusted without
    /// suppressing it.
    ///
    /// Returns status in bits 0..7, active-alert count in bits 8..15,
    /// faulted health in bit 16, overflow in bit 17, and the manager
    /// generation in bits 32..63.
    pub fn step_alerts(&mut self, now_ms: u64, path_healthy: u32) -> u64 {
        self.runtime.as_mut().map_or_else(
            || pack_alerts(AlertStepOutcome::failure(RenderStatus::NotInitialized)),
            |runtime| pack_alerts(runtime.step_alerts(now_ms, path_healthy != 0)),
        )
    }

    /// The controlled glyph pack's canonical serialization (REN-02), for
    /// the backend's independent hash verification and atlas
    /// construction. A serialization failure returns an empty buffer,
    /// which no verifier accepts.
    pub fn glyph_manifest(&self) -> Vec<u8> {
        pilotage_instrument_runtime::glyph_manifest()
    }

    /// The compile-time-recorded glyph content hash the backend must
    /// match against both the canonical bytes and its own pinned value.
    pub fn glyph_recorded_hash(&self) -> Vec<u8> {
        pilotage_instrument_runtime::glyph_recorded_hash()
    }
}

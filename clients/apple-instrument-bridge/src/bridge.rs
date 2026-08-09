//! The bridge object: one owned instrument runtime behind the FFI.
//! The caller copies each state frame in, then asks for panel scenes.

use std::sync::Mutex;

use pilotage_instrument_runtime::{RenderStatus, Runtime, canonical_frame, descriptor};

use crate::records::{BridgeRenderOutcome, BridgeWriteOutcome};

/// One owned instrument runtime behind the FFI. Each bridge object has
/// independent buffers, configuration, and generations.
#[derive(uniffi::Object)]
pub struct InstrumentBridge {
    runtime: Mutex<Runtime>,
}

impl Default for InstrumentBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[uniffi::export]
impl InstrumentBridge {
    /// Constructs a bridge with a fresh runtime.
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self {
            runtime: Mutex::new(Runtime::new()),
        }
    }

    /// Copies one ABI state frame into the runtime's state buffer.
    /// The function refuses input that exceeds the fixed capacity.
    pub fn write_state(&self, bytes: &[u8]) -> BridgeWriteOutcome {
        let mut runtime = match self.runtime.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let buffer = runtime.state_mut();
        buffer.fill(0);
        if bytes.len() > buffer.len() {
            return BridgeWriteOutcome {
                status: 1,
                actual: bytes.len() as u64,
                capacity: buffer.len() as u64,
            };
        }
        if let Some(target) = buffer.get_mut(..bytes.len()) {
            target.copy_from_slice(bytes);
        }
        BridgeWriteOutcome {
            status: 0,
            actual: bytes.len() as u64,
            capacity: buffer.len() as u64,
        }
    }

    /// Renders one panel from the last written state frame.
    ///
    /// The runtime draws every panel at its canonical frame. The outcome
    /// returns this frame so the caller can compare it with the frame that
    /// its display backend requested.
    ///
    /// A failure carries a nonzero status, empty scene bytes, and an
    /// unchanged generation.
    pub fn render(&self, panel: u32) -> BridgeRenderOutcome {
        let mut runtime = match self.runtime.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let outcome = runtime.render(panel);
        let (emitted_width, emitted_height) = descriptor(panel)
            .map(|d| {
                let frame = canonical_frame(d);
                (frame.width, frame.height)
            })
            .unwrap_or((0.0, 0.0));
        let scene = if outcome.status == RenderStatus::Ok {
            runtime
                .scene()
                .get(..outcome.scene_len as usize)
                .map_or_else(Vec::new, <[u8]>::to_vec)
        } else {
            Vec::new()
        };
        BridgeRenderOutcome {
            status: outcome.status as u32,
            scene,
            frame_width: emitted_width,
            frame_height: emitted_height,
            generation: outcome.generation,
        }
    }
}

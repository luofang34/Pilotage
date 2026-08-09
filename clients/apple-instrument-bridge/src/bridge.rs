//! The bridge object for one clocked composition transaction.

use std::sync::Mutex;

use pilotage_instrument_runtime::{RenderStatus, Runtime};

use crate::records::{
    BridgeCompositionFrameOutcome, BridgeCompositionPanelOutcome, BridgeWriteOutcome,
};

struct BridgeState {
    runtime: Runtime,
    accepted_at_ms: Option<u64>,
}

/// One owned instrument runtime behind the FFI. Each bridge object has
/// independent buffers, configuration, and generations.
#[derive(uniffi::Object)]
pub struct InstrumentBridge {
    state: Mutex<BridgeState>,
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
            state: Mutex::new(BridgeState {
                runtime: Runtime::new(),
                accepted_at_ms: None,
            }),
        }
    }

    /// Copies one state frame and records its monotonic acceptance time.
    pub fn write_state(&self, bytes: &[u8], accepted_at_ms: u64) -> BridgeWriteOutcome {
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.accepted_at_ms = None;
        let capacity = Runtime::state_capacity();
        state.runtime.state_mut().fill(0);
        if bytes.len() > capacity {
            return BridgeWriteOutcome {
                status: 1,
                actual: bytes.len() as u64,
                capacity: capacity as u64,
            };
        }
        if let Some(target) = state.runtime.state_mut().get_mut(..bytes.len()) {
            target.copy_from_slice(bytes);
        }
        state.accepted_at_ms = Some(accepted_at_ms);
        BridgeWriteOutcome {
            status: 0,
            actual: bytes.len() as u64,
            capacity: capacity as u64,
        }
    }

    /// Produces all composition panels from one state and one clock.
    pub fn composition_frame(
        &self,
        now_ms: u64,
        path_healthy: bool,
    ) -> BridgeCompositionFrameOutcome {
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let age_delta_ms = state
            .accepted_at_ms
            .map_or(0, |accepted_at_ms| now_ms.saturating_sub(accepted_at_ms));
        let outcome = state
            .runtime
            .render_composition(age_delta_ms, now_ms, path_healthy);
        let scene = if outcome.status == RenderStatus::Ok {
            state
                .runtime
                .composition_scene()
                .get(..outcome.scene_len as usize)
                .map_or_else(Vec::new, <[u8]>::to_vec)
        } else {
            Vec::new()
        };
        let panels = state
            .runtime
            .composition_panel_outcomes()
            .iter()
            .map(|panel| BridgeCompositionPanelOutcome {
                panel: panel.panel,
                status: panel.status as u32,
                scene_offset: panel.scene_offset,
                scene_len: panel.scene_len,
                frame_width: panel.frame_width,
                frame_height: panel.frame_height,
                generation: panel.generation,
            })
            .collect();
        BridgeCompositionFrameOutcome {
            status: outcome.status as u32,
            scene,
            panels,
            generation: outcome.generation,
            alert_status: outcome.alerts.status as u32,
            active_alert_count: outcome.alerts.active_count,
            alert_path_faulted: outcome.alerts.faulted,
            alert_overflow: outcome.alerts.overflow,
            alert_manager_generation: outcome.alerts.manager_generation,
        }
    }
}

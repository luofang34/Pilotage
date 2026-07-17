//! Flight-control input helpers and reset handling.

use std::time::{Duration, Instant};

use pilotage_adapter_api::{ApplyOutcome, Disposition, RejectReason};
use pilotage_protocol::{ButtonEdge, LogicalButtonId, ScopedControlFrame};
use pilotage_timing::SimTick;

use super::AviateAdapter;

pub(super) fn flight_button_pressed(frame: &ScopedControlFrame, button: u16) -> bool {
    frame.payload.edges.iter().any(|(candidate, edge)| {
        *edge == ButtonEdge::Pressed && *candidate == LogicalButtonId::new(button)
    })
}

pub(super) fn rejected_control(tick: SimTick, reason: RejectReason) -> ApplyOutcome {
    ApplyOutcome {
        tick,
        disposition: Disposition::Rejected(reason),
    }
}

pub(super) fn normalized_flight_sticks(frame: &ScopedControlFrame) -> ([f32; 4], bool) {
    let mut sticks = [0.0_f32; 4];
    let mut transformed = false;
    for (axis, value) in &frame.payload.axes {
        let clamped = if value.is_nan() {
            0.0
        } else {
            value.clamp(-1.0, 1.0)
        };
        transformed |= clamped != *value;
        sticks[usize::from(axis.as_u16().min(3))] = clamped;
    }
    (sticks, transformed)
}

impl AviateAdapter {
    /// Runs the SITL reset script (debounced to one per 5 s): world
    /// reset + FC restart, fire-and-forget. `PILOTAGE_RESET_CMD`
    /// overrides the script path.
    pub(super) fn spawn_reset(&mut self) {
        let now = Instant::now();
        if self
            .last_reset
            .is_some_and(|last_reset| now.duration_since(last_reset) < Duration::from_secs(5))
        {
            return;
        }
        self.last_reset = Some(now);
        // The restarted FC re-reports arm state under fresh heartbeats;
        // the pre-reset report must not survive as current.
        self.arm = None;
        let script = std::env::var("PILOTAGE_RESET_CMD").unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(std::path::Path::parent)
                .map_or_else(|| ".".to_owned(), |path| path.display().to_string())
                + "/scripts/reset-flight-sim.sh"
        });
        tracing::info!(%script, "simulation reset requested from the viewer");
        if let Err(error) = std::process::Command::new(&script)
            .arg("aviate_sitl")
            .spawn()
        {
            tracing::warn!(%error, %script, "reset script failed to spawn");
        }
    }
}

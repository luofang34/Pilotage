//! Flight-control reset handling.

use std::time::{Duration, Instant};

use super::AviateAdapter;

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
        self.armed = None;
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

//! The shell-facing callback surface.

use super::records::LinkEvent;

/// What the shell implements to receive the link's output. Every call
/// arrives from a background task; the implementation owns its own
/// dispatch to the interface thread.
#[uniffi::export(with_foreign)]
pub trait LinkObserver: Send + Sync {
    /// One typed link event.
    fn on_event(&self, event: LinkEvent);
    /// One encoded instrument state frame, assembled by the shared feed
    /// from admitted telemetry. `accepted_at_ms` is the link's monotonic
    /// clock at assembly.
    fn on_state_frame(&self, frame: Vec<u8>, accepted_at_ms: u64);
}

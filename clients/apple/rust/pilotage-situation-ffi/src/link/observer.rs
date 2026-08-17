//! The shell-facing callback surface.

use super::records::LinkEvent;

/// What the shell implements to receive the link's output. Every call
/// arrives on the link's own delivery thread; the implementation owns
/// its own dispatch to the interface thread. The link brackets every
/// delivery batch in an autorelease pool, so an implementation may call
/// Objective-C frameworks freely — long work still belongs off this
/// thread, because the next batch waits for it.
#[uniffi::export(with_foreign)]
pub trait LinkObserver: Send + Sync {
    /// One typed link event.
    fn on_event(&self, event: LinkEvent);
    /// One encoded instrument state frame, assembled by the shared feed
    /// from admitted telemetry. `accepted_at_ms` is the link's monotonic
    /// clock at assembly.
    fn on_state_frame(&self, frame: Vec<u8>, accepted_at_ms: u64);
    /// One decoded video frame: which source, which codec (a FourCC such
    /// as "H264" or "MJPG"), and the encoded payload. The platform owns
    /// the decoder and the surface (ADR-0037).
    fn on_video_frame(&self, source_id: u8, codec: String, payload: Vec<u8>);
}

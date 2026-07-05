//! Timing-staleness conformance: an artificially aged control frame must be
//! rejected by the `pilotage-timing` staleness policy (ADR-0009).
//!
//! This is the time-domain complement to the authority engine's
//! generation-fencing: even a frame at the current generation is dropped if
//! it arrives older than the configured maximum control age. The harness
//! ages a frame by holding a fixed `sampled_at` while advancing the host's
//! `now`, then classifies the elapsed age against a [`StalenessPolicy`].

use core::time::Duration;

use pilotage_protocol::ScopedControlFrame;
use pilotage_timing::{Freshness, MonoTimestamp, StalenessPolicy};

/// Classifies `frame` against `policy` given the host's current time `now`.
///
/// The frame's age is `now - frame.sampled_at` in the host's monotonic
/// domain (the harness treats `sampled_at` and `now` as the same endpoint's
/// clock, so no offset correlation is needed). Returns the [`Freshness`] the
/// policy assigns, so callers can assert a fresh frame stays fresh and an
/// aged one is rejected as [`Freshness::Stale`].
#[must_use]
pub fn aged_frame_is_stale(
    frame: &ScopedControlFrame,
    now: MonoTimestamp,
    policy: &StalenessPolicy,
) -> Freshness {
    let age: Duration = now.saturating_duration_since(frame.sampled_at);
    policy.check(age)
}

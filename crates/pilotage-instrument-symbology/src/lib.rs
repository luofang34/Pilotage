//! Shared instrument symbology: how every panel says what it knows and,
//! more importantly, what it does not (ADR-0029).
//!
//! Panels depend on this crate; it depends on no panel. Everything here is
//! the vocabulary a display uses to be honest about signal trust — the
//! cockpit palette, the flag/dash/red-X paint for non-`Valid` statuses,
//! the manager-driven alert stack, per-function source labels, and the
//! no-alloc label formatter. Keeping it in one place means two panels can
//! never answer "how does this display say the value is not trustworthy?"
//! differently.
//!
//! ADR-0029 names the never-skinnable set — failure pages, flag colors,
//! alert semantics, required-layer isolation. Housing the shared
//! symbology in one crate is what lets that boundary be enforced
//! structurally rather than documented.

#![no_std]

pub mod annunciation;
pub mod fixed_str;
pub mod palette;
pub mod source_label;
pub mod status_paint;

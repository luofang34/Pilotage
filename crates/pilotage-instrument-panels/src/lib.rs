//! PFD and HSI panels as pure state→scene functions (ADR-0017).
//!
//! Each panel is a function from resolved display state
//! ([`pilotage_instrument_state::PanelData`]) to abstract drawing commands
//! ([`pilotage_instrument_scene::SceneWriter`]); no panel knows what
//! renders it. Panels draw in a fixed logical space of
//! [`PANEL_W`]×[`PANEL_H`] units (the Garmin-G5 proportions the geometry
//! constants come from); backends scale that space to their viewport.
//!
//! Signal statuses are honored, never hidden: `Missing` renders dashes,
//! `Stale`/`Degraded` render amber flags, `Failed` renders a red X in
//! place of the instrument (the pyG5 reference's single avionics-off flag
//! is exactly the shortfall this replaces).

#![no_std]

#[cfg(test)]
extern crate std;

mod annunciation;
mod fixed_str;
mod hsi;
mod palette;
mod pfd;
mod status_paint;

pub use hsi::draw_hsi;
pub use pfd::{BackgroundMode, PfdConfig, VSpeeds, draw_pfd};

/// Logical panel width all panels draw against.
pub const PANEL_W: f32 = 480.0;

/// Logical panel height all panels draw against.
pub const PANEL_H: f32 = 360.0;

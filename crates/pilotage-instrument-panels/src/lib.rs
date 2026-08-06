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

#[cfg(test)]
mod alert_stack_tests;
mod descriptors;
mod hsi;
mod monitor;
mod pfd;

pub use descriptors::{
    BUILTIN_PANELS, BUILTIN_SCENE_DIGEST, HSI_DESCRIPTOR, MONITOR_DESCRIPTOR, PFD_DESCRIPTOR,
};
pub use hsi::draw_hsi;
pub use monitor::draw_monitor;
pub use pfd::{BackgroundMode, PFD_CONFIG_SCHEMA, PfdConfig, SvsViewport, VSpeeds, draw_pfd};

/// Logical panel width all panels draw against.
pub const PANEL_W: f32 = 480.0;

/// Logical panel height all panels draw against.
pub const PANEL_H: f32 = 360.0;

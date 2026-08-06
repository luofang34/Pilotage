//! Cockpit color conventions (Garmin-style, mined from the pyG5 spec).
//!
//! Magenta marks GPS/derived guidance; green marks radio-nav; cyan marks
//! pilot selections (bugs); white is primary data; amber flags degraded
//! or stale signals; red marks failure and limits.
//!
//! `RED`, `AMBER`, `YELLOW`, and `BAND_YELLOW` are the [`crate::safety`]
//! set under their conventional names. `WHITE` stays an independent
//! constant because it is dual-role — the themable primary default and
//! the advisory-row color — which is also why white is exempt from the
//! theme imitation rule. The remaining constants are the default values
//! of the themable set ([`crate::theme::Theme`]).

use pilotage_instrument_scene::Rgba8;

use crate::safety;

/// Sky half of the attitude ball.
pub const SKY: Rgba8 = Rgba8::rgb(0, 110, 210);

/// Ground half of the attitude ball.
pub const GROUND: Rgba8 = Rgba8::rgb(140, 96, 44);

/// Primary symbology and scale marks.
pub const WHITE: Rgba8 = Rgba8::rgb(255, 255, 255);

/// Panel background.
pub const BLACK: Rgba8 = Rgba8::rgb(0, 0, 0);

/// Semi-transparent tape/box background over the horizon.
pub const TAPE_BG: Rgba8 = Rgba8::rgba(20, 20, 20, 150);

/// Solid readout-box background.
pub const BOX_BG: Rgba8 = Rgba8::rgb(0, 0, 0);

/// Box outlines and secondary marks.
pub const GREY: Rgba8 = Rgba8::rgb(128, 128, 128);

/// GPS guidance, trends, and rate cues.
pub const MAGENTA: Rgba8 = Rgba8::rgb(255, 0, 255);

/// Radio-nav (VLOC) guidance.
pub const GREEN: Rgba8 = Rgba8::rgb(0, 255, 0);

/// Pilot selections: bugs, selected values, baro.
pub const CYAN: Rgba8 = Rgba8::rgb(0, 255, 255);

/// The fixed aircraft reference symbol and caution band.
pub const YELLOW: Rgba8 = safety::REFERENCE_YELLOW;

/// Degraded/stale signal flags.
pub const AMBER: Rgba8 = safety::CAUTION_AMBER;

/// Failure flags and the never-exceed band.
pub const RED: Rgba8 = safety::FAILURE_RED;

/// Normal-range band on the speed tape.
pub const BAND_GREEN: Rgba8 = Rgba8::rgb(0, 160, 0);

/// Caution-range band on the speed tape.
pub const BAND_YELLOW: Rgba8 = safety::BAND_CAUTION;

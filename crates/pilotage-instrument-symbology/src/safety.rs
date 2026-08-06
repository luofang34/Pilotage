//! The never-skinnable colors (ADR-0029): failure, caution, alert-class,
//! and limit semantics.
//!
//! These constants are reachable from no configuration path — [`Theme`]
//! has no field that can express or override them, and
//! [`crate::theme::ValidTheme`] additionally rejects any themable color
//! close enough to imitate one. A theme is therefore *unable* to change
//! what `Missing`, `Stale`, `Degraded`, or `Failed` paints, rather than
//! being asked not to.
//!
//! [`Theme`]: crate::theme::Theme

use pilotage_instrument_scene::Rgba8;

/// Failure flags, the red X, dashes behind hidden values, and the
/// never-exceed band.
pub const FAILURE_RED: Rgba8 = Rgba8::rgb(255, 0, 0);

/// Degraded/stale flags, caution alert rows, and reversion/miscompare
/// source labels.
pub const CAUTION_AMBER: Rgba8 = Rgba8::rgb(255, 176, 0);

/// Advisory and lower alert rows in the annunciation stack.
pub const ANNUNCIATION_WHITE: Rgba8 = Rgba8::rgb(255, 255, 255);

/// The fixed aircraft reference symbol and the caution band edge color.
pub const REFERENCE_YELLOW: Rgba8 = Rgba8::rgb(255, 255, 0);

/// Caution-range band on the speed tape.
pub const BAND_CAUTION: Rgba8 = Rgba8::rgb(230, 200, 0);

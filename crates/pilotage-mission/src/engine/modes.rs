//! The state of the engine's mode machines, kept together so each machine
//! owns its own state and none reads another's.

use super::lateral_mode::Lateral;
use super::lifecycle::Lifecycle;
use super::nav_source::NavMode;
use super::vertical_mode::VerticalStatus;

/// The state of each mode machine after the latest tick.
#[derive(Debug, Default)]
pub(super) struct Modes {
    /// The navigation source and its integrity.
    pub(super) nav: NavMode,
    /// The mission lifecycle.
    pub(super) lifecycle: Lifecycle,
    /// The lateral mode.
    pub(super) lateral: Lateral,
    /// The vertical mode.
    pub(super) vertical: VerticalStatus,
}

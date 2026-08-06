//! The panel descriptor: everything a shell needs to wire one panel.

use pilotage_alerts::AlertOutput;
use pilotage_instrument_scene::{SceneError, SceneWriter};
use pilotage_instrument_state::{AircraftState, GroupId, PanelData};

use crate::config::{ConfigBlob, ConfigError, ConfigKey};
use crate::group_set::GroupSet;

/// A panel's draw entry point: pure resolved-state → scene, with its
/// configuration delivered as the validated blob the shell accepted.
pub type DrawFn = fn(
    &PanelData,
    &ConfigBlob<'_>,
    Option<&AlertOutput>,
    &mut SceneWriter<'_>,
) -> Result<(), PanelDrawError>;

/// Why a panel draw failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PanelDrawError {
    /// The scene writer refused a command. The typed reason rides
    /// along; [`SceneError`] carries no `Display` of its own.
    #[error("scene writer refused a command")]
    Scene(SceneError),
    /// The configuration blob does not decode for this panel.
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
}

impl From<SceneError> for PanelDrawError {
    fn from(error: SceneError) -> Self {
        PanelDrawError::Scene(error)
    }
}

/// What the panel does with the `Background` scene band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundCapability {
    /// Never draws the band (a compositor may still not use it).
    NotUsed,
    /// Owns the band with opaque content of its own.
    Opaque,
    /// Draws the band by default but cedes it on request — the panel
    /// stays complete when the band is supplied elsewhere (SVS, video).
    Cedeable,
}

/// The logical space a panel draws against; backends scale, panels
/// never see viewport pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignFrame {
    /// Logical width.
    pub width: f32,
    /// Logical height.
    pub height: f32,
}

/// An axis-aligned rectangle in design-frame units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// A panel-contributed extreme state for conformance and digest runs:
/// the panel names the situations that stress it beyond the shared
/// canonical set.
#[derive(Debug, Clone, Copy)]
pub struct ExtremeState {
    /// Stable identity of the fixture (lowercase, digits, dashes).
    pub id: &'static str,
    /// Builds the state; a plain fn keeps descriptors `static`.
    pub build: fn() -> AircraftState,
}

/// One panel, as data. A shell composes descriptors into a
/// [`crate::Registry`] and consumes only this — no shell may hold a
/// panel list, index, or layer mask of its own (ADR-0029).
#[derive(Debug, Clone, Copy)]
pub struct PanelDescriptor {
    /// Stable identity (lowercase, digits, dashes): canvas ids, health
    /// keys, and evidence records key off this.
    pub id: &'static str,
    /// Operator-facing title.
    pub title: &'static str,
    /// Scene layers that must be present and complete in every frame,
    /// as a bitset over [`pilotage_instrument_scene::LayerId`].
    pub required_layers: u8,
    /// State groups this panel consumes — the withholding matrix the
    /// admission harness drives honest-status checks from.
    pub required_groups: GroupSet,
    /// The logical space this panel draws against.
    pub design_frame: DesignFrame,
    /// What the panel does with the `Background` band.
    pub background: BackgroundCapability,
    /// Configuration keys this panel understands; a shell refuses a
    /// blob carrying any other key.
    pub config_schema: &'static [ConfigKey],
    /// Where each consumed group paints, for honest-status region
    /// checks; populated when the admission harness consumes it.
    pub group_regions: &'static [(GroupId, Region)],
    /// Panel-contributed stress fixtures beyond the canonical states.
    pub extreme_states: &'static [ExtremeState],
    /// Pinned raster baseline: SHA-256 hex of the reference
    /// rasterizer's render of the shared "typical" corpus state, or
    /// `None` until the baseline travels into the descriptor.
    pub raster_baseline: Option<&'static str>,
    /// The draw entry point.
    pub draw: DrawFn,
}

//! The cross-shell scene digest (ADR-0033): one number that proves two
//! shells show the same instruments.
//!
//! The digest streams, per registered panel: the role-tagged,
//! length-prefixed panel id and the contract-relevant descriptor
//! fields, then per corpus state the role-tagged state id and emitted
//! scene bytes — drawn with the empty config and no alerts, so it is
//! invariant to SVS by construction (theme independence holds because
//! panels take no theme parameter at this boundary). Shells report the same digest or they are
//! not showing the same panels; pixel hashes stay per-backend
//! rasterizer regression tests, not the cross-shell contract. The
//! digest moves exactly once per deliberate contract change, re-pinned
//! with a review note saying why.

use pilotage_instrument_scene::{SCENE_FORMAT_VERSION, SceneWriter};
use pilotage_instrument_state::{FreshnessPolicy, abi::v6, resolve};
use pilotage_sha256::Sha256Ctx;

use crate::config::EMPTY_CONFIG;
use crate::descriptor::{PanelDescriptor, PanelDrawError};
use crate::registry::Registry;
use crate::states::CANONICAL_STATES;

/// Domain separator; a new value is a deliberate contract break.
pub const SCENE_DIGEST_DOMAIN: &[u8] = b"pilotage-scene-digest-v1";

/// Why a digest run failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DigestError {
    /// A panel refused to draw a corpus state.
    #[error("panel {panel} failed to draw corpus state {state}")]
    Draw {
        /// The refusing panel.
        panel: &'static str,
        /// The corpus state being drawn.
        state: &'static str,
        /// The panel's own reason.
        #[source]
        source: PanelDrawError,
    },
    /// The caller's scratch buffer cannot hold a scene.
    #[error("scene scratch buffer of {len} bytes is too small")]
    Scratch {
        /// The offending buffer length.
        len: usize,
    },
}

/// Item-role tags framing the digest stream: every item carries its
/// role, so no cross-role collision can exist even in principle.
const ROLE_PANEL: u8 = 1;
const ROLE_STATE: u8 = 2;
const ROLE_SCENE: u8 = 3;

/// Digests `registry` over the shared corpus plus each panel's own
/// extreme states, drawing into `scratch` (size it
/// [`pilotage_instrument_scene::MAX_SCENE_BYTES`]).
pub fn scene_digest(registry: &Registry, scratch: &mut [u8]) -> Result<[u8; 32], DigestError> {
    let mut ctx = Sha256Ctx::new();
    ctx.update(SCENE_DIGEST_DOMAIN);
    ctx.update(&[SCENE_FORMAT_VERSION, v6::VERSION]);
    for panel in registry.panels() {
        digest_panel_contract(&mut ctx, panel);
        for state in CANONICAL_STATES {
            digest_state(&mut ctx, panel, state.id, (state.build)(), scratch)?;
        }
        for extreme in panel.extreme_states {
            digest_state(&mut ctx, panel, extreme.id, (extreme.build)(), scratch)?;
        }
    }
    Ok(ctx.finalize())
}

/// Binds the contract-relevant descriptor fields, not just the id: two
/// shells whose descriptors declare different required layers, groups,
/// frames, background capability, or schemas are not showing the same
/// instruments even if their scene bytes agree.
fn digest_panel_contract(ctx: &mut Sha256Ctx, panel: &PanelDescriptor) {
    update_framed(ctx, ROLE_PANEL, panel.id.as_bytes());
    ctx.update(&[panel.required_layers]);
    ctx.update(&panel.required_groups.bits().to_le_bytes());
    ctx.update(&panel.design_frame.width.to_le_bytes());
    ctx.update(&panel.design_frame.height.to_le_bytes());
    ctx.update(&[match panel.background {
        crate::descriptor::BackgroundCapability::NotUsed => 0,
        crate::descriptor::BackgroundCapability::Opaque => 1,
        crate::descriptor::BackgroundCapability::Cedeable => 2,
    }]);
    ctx.update(&(panel.config_schema.len() as u32).to_le_bytes());
    for key in panel.config_schema {
        ctx.update(&key.0.to_le_bytes());
    }
}

fn digest_state(
    ctx: &mut Sha256Ctx,
    panel: &PanelDescriptor,
    state_id: &'static str,
    state: pilotage_instrument_state::AircraftState,
    scratch: &mut [u8],
) -> Result<(), DigestError> {
    update_framed(ctx, ROLE_STATE, state_id.as_bytes());
    let data = resolve(&state, &FreshnessPolicy::default());
    let scratch_len = scratch.len();
    let mut writer =
        SceneWriter::new(scratch).map_err(|_| DigestError::Scratch { len: scratch_len })?;
    (panel.draw)(&data, &EMPTY_CONFIG, None, &mut writer).map_err(|source| DigestError::Draw {
        panel: panel.id,
        state: state_id,
        source,
    })?;
    let used = writer.finish();
    let Some(scene) = scratch.get(..used) else {
        // A writer that reports more bytes than its buffer is broken;
        // digesting a truncated scene would silently misstate identity.
        return Err(DigestError::Scratch { len: scratch_len });
    };
    update_framed(ctx, ROLE_SCENE, scene);
    Ok(())
}

/// Role-tagged, length-prefixed (`u32` LE) update: framing keeps
/// adjacent fields and different item roles from aliasing each other.
fn update_framed(ctx: &mut Sha256Ctx, role: u8, bytes: &[u8]) {
    ctx.update(&[role]);
    ctx.update(&(bytes.len() as u32).to_le_bytes());
    ctx.update(bytes);
}

// The digest pin over the shipped panels lives in the panels crate
// (`descriptors/digest_tests.rs`): a dev-dependency back onto panels
// would duplicate this crate in the test graph and split its types.
#[cfg(test)]
mod tests;

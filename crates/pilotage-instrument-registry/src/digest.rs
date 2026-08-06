//! The cross-shell scene digest (ADR-0033): one number that proves two
//! shells show the same instruments.
//!
//! The digest streams, per registered panel and corpus state, the
//! length-prefixed panel id, state id, and emitted scene bytes — drawn
//! with the empty config and no alerts, so it is invariant to SVS and
//! themes by construction. Shells report the same digest or they are
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

/// Digests `registry` over the shared corpus plus each panel's own
/// extreme states, drawing into `scratch` (size it
/// [`pilotage_instrument_scene::MAX_SCENE_BYTES`]).
pub fn scene_digest(registry: &Registry, scratch: &mut [u8]) -> Result<[u8; 32], DigestError> {
    let mut ctx = Sha256Ctx::new();
    ctx.update(SCENE_DIGEST_DOMAIN);
    ctx.update(&[SCENE_FORMAT_VERSION, v6::VERSION]);
    for panel in registry.panels() {
        update_framed(&mut ctx, panel.id.as_bytes());
        for state in CANONICAL_STATES {
            digest_state(&mut ctx, panel, state.id, (state.build)(), scratch)?;
        }
        for extreme in panel.extreme_states {
            digest_state(&mut ctx, panel, extreme.id, (extreme.build)(), scratch)?;
        }
    }
    Ok(ctx.finalize())
}

fn digest_state(
    ctx: &mut Sha256Ctx,
    panel: &PanelDescriptor,
    state_id: &'static str,
    state: pilotage_instrument_state::AircraftState,
    scratch: &mut [u8],
) -> Result<(), DigestError> {
    update_framed(ctx, state_id.as_bytes());
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
    update_framed(ctx, scratch.get(..used).unwrap_or(&[]));
    Ok(())
}

/// Length-prefixed (`u32` LE) update: framing keeps adjacent fields
/// from aliasing each other's bytes.
fn update_framed(ctx: &mut Sha256Ctx, bytes: &[u8]) {
    ctx.update(&(bytes.len() as u32).to_le_bytes());
    ctx.update(bytes);
}

// The digest pin over the shipped panels lives in the panels crate
// (`descriptors/digest_tests.rs`): a dev-dependency back onto panels
// would duplicate this crate in the test graph and split its types.
#[cfg(test)]
mod tests;

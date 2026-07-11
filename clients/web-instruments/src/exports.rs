//! The `extern "C"` surface and its process-global context.
//!
//! Failure semantics (DISP-01): [`render_status`] never reports success
//! without a complete, structurally validated scene. The scene buffer's
//! contents after a failed attempt are unspecified — consumers must read
//! scene bytes only after a `0` status, and only [`scene_len`] bytes.

use std::sync::Mutex;

use pilotage_instrument_panels::{PfdConfig, VSpeeds, draw_hsi, draw_pfd};
use pilotage_instrument_scene::{SceneCmds, SceneError, SceneWriter};
use pilotage_instrument_state::abi::{AbiError, STATE_ABI_SIZE, STATE_ABI_VERSION, decode_state};
use pilotage_instrument_state::{FreshnessPolicy, resolve};

use crate::render_status::RenderStatus;

const SCENE_CAPACITY: usize = 64 * 1024;

/// Panel id JS passes to [`render_status`].
const PANEL_PFD: u32 = 0;
/// Panel id JS passes to [`render_status`].
const PANEL_HSI: u32 = 1;
/// Panels with independently tracked success generations.
const PANEL_COUNT: usize = 2;

pub(crate) struct Ctx {
    pub(crate) state: Vec<u8>,
    pub(crate) scene: Vec<u8>,
    /// Byte length of the last successfully rendered scene.
    pub(crate) scene_len: usize,
    /// Per-panel wrapping count of successful renders. A failed attempt
    /// never advances a generation, so a stalled generation is a
    /// watchdog signal, not a rendering hiccup.
    pub(crate) generation: [u32; PANEL_COUNT],
    pub(crate) pfd_cfg: PfdConfig,
}

pub(crate) static CTX: Mutex<Option<Ctx>> = Mutex::new(None);

fn scene_error_status(error: SceneError) -> RenderStatus {
    match error {
        SceneError::BufferFull => RenderStatus::SceneBufferFull,
        SceneError::TooManyPoints | SceneError::TextTooLong => RenderStatus::SceneCommandLimit,
    }
}

/// Whether every command in `scene` decodes cleanly; a partially
/// paintable but structurally broken scene must never reach a backend.
fn scene_is_structurally_valid(scene: &[u8]) -> bool {
    match SceneCmds::new(scene) {
        Ok(mut cmds) => cmds.all(|cmd| cmd.is_ok()),
        Err(_) => false,
    }
}

pub(crate) fn render_into(ctx: &mut Ctx, panel: u32) -> RenderStatus {
    let panel_idx = match panel {
        PANEL_PFD => 0usize,
        PANEL_HSI => 1,
        _ => return RenderStatus::InvalidPanel,
    };
    let state = match decode_state(&ctx.state) {
        Ok(state) => state,
        Err(AbiError::Truncated) => return RenderStatus::StateTruncated,
        Err(AbiError::BadVersion { .. }) => return RenderStatus::StateBadVersion,
    };
    let data = resolve(&state, &FreshnessPolicy::default());
    let mut writer = match SceneWriter::new(&mut ctx.scene) {
        Ok(writer) => writer,
        Err(error) => return scene_error_status(error),
    };
    let drawn = if panel == PANEL_PFD {
        draw_pfd(&data, &ctx.pfd_cfg, &mut writer)
    } else {
        draw_hsi(&data, &mut writer)
    };
    let len = match drawn {
        Ok(()) => writer.finish(),
        Err(error) => return scene_error_status(error),
    };
    if !scene_is_structurally_valid(ctx.scene.get(..len).unwrap_or(&[])) {
        return RenderStatus::SceneStructure;
    }
    ctx.scene_len = len;
    if let Some(generation) = ctx.generation.get_mut(panel_idx) {
        *generation = generation.wrapping_add(1);
    }
    RenderStatus::Ok
}

/// The state-block ABI version this module was built against.
#[allow(unsafe_code)] // SAFETY: `#[unsafe(no_mangle)]` only marks the symbol exported; the function body is safe code.
#[unsafe(no_mangle)]
pub extern "C" fn abi_version() -> u32 {
    STATE_ABI_VERSION
}

/// Allocates the state and scene buffers; returns 1 on success.
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn init() -> u32 {
    let Ok(mut ctx) = CTX.lock() else {
        return 0;
    };
    *ctx = Some(Ctx {
        state: vec![0u8; STATE_ABI_SIZE],
        scene: vec![0u8; SCENE_CAPACITY],
        scene_len: 0,
        generation: [0; PANEL_COUNT],
        pfd_cfg: PfdConfig::default(),
    });
    1
}

/// Linear-memory offset of the packed state block JS writes.
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn state_ptr() -> u32 {
    let Ok(ctx) = CTX.lock() else {
        return 0;
    };
    ctx.as_ref().map_or(0, |c| c.state.as_ptr() as u32)
}

/// Size of the packed state block in bytes.
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn state_len() -> u32 {
    STATE_ABI_SIZE as u32
}

/// Linear-memory offset of the encoded scene JS reads back.
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn scene_ptr() -> u32 {
    let Ok(ctx) = CTX.lock() else {
        return 0;
    };
    ctx.as_ref().map_or(0, |c| c.scene.as_ptr() as u32)
}

/// Byte length of the most recent successfully rendered scene; `0` when
/// no render has succeeded since [`init`].
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn scene_len() -> u32 {
    let Ok(ctx) = CTX.lock() else {
        return 0;
    };
    ctx.as_ref().map_or(0, |c| c.scene_len as u32)
}

/// Wrapping count of successful renders for `panel` (0 = PFD, 1 = HSI);
/// `0` for unknown panels. Failed attempts never advance it, so a
/// consumer watchdog can distinguish "rendering but failing" from
/// "renderer stalled".
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn render_generation(panel: u32) -> u32 {
    let Ok(ctx) = CTX.lock() else {
        return 0;
    };
    ctx.as_ref().map_or(0, |c| {
        usize::try_from(panel)
            .ok()
            .and_then(|idx| c.generation.get(idx).copied())
            .unwrap_or(0)
    })
}

/// Configures speed-tape bands (knots); pass all zeros to clear.
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn set_v_speeds(vs0: f32, vs: f32, vfe: f32, vno: f32, vne: f32) {
    let Ok(mut ctx) = CTX.lock() else {
        return;
    };
    if let Some(c) = ctx.as_mut() {
        c.pfd_cfg.v_speeds = if vne > 0.0 {
            Some(VSpeeds {
                vs0_kt: vs0,
                vs_kt: vs,
                vfe_kt: vfe,
                vno_kt: vno,
                vne_kt: vne,
            })
        } else {
            None
        };
    }
}

/// Renders panel `panel` (0 = PFD, 1 = HSI) from the current state block
/// and returns a [`RenderStatus`] code. On `0` the scene bytes are at
/// [`scene_ptr`]`..`[`scene_len`] and the panel's [`render_generation`]
/// has advanced; on any other code the scene buffer contents are
/// unspecified and must not be painted.
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn render_status(panel: u32) -> u32 {
    let Ok(mut guard) = CTX.lock() else {
        return RenderStatus::ContextUnavailable as u32;
    };
    let Some(ctx) = guard.as_mut() else {
        return RenderStatus::NotInitialized as u32;
    };
    render_into(ctx, panel) as u32
}

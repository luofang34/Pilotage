//! The `extern "C"` surface and its process-global context.

use std::sync::Mutex;

use pilotage_instrument_panels::{PfdConfig, VSpeeds, draw_hsi, draw_pfd};
use pilotage_instrument_scene::SceneWriter;
use pilotage_instrument_state::abi::{STATE_ABI_SIZE, STATE_ABI_VERSION, decode_state};
use pilotage_instrument_state::{FreshnessPolicy, resolve};

const SCENE_CAPACITY: usize = 64 * 1024;

/// Panel id JS passes to [`render`].
const PANEL_PFD: u32 = 0;
/// Panel id JS passes to [`render`].
const PANEL_HSI: u32 = 1;

pub(crate) struct Ctx {
    pub(crate) state: Vec<u8>,
    pub(crate) scene: Vec<u8>,
    pfd_cfg: PfdConfig,
}

pub(crate) static CTX: Mutex<Option<Ctx>> = Mutex::new(None);

fn render_into(ctx: &mut Ctx, panel: u32) -> usize {
    let Ok(state) = decode_state(&ctx.state) else {
        return 0;
    };
    let data = resolve(&state, &FreshnessPolicy::default());
    let Ok(mut writer) = SceneWriter::new(&mut ctx.scene) else {
        return 0;
    };
    let drawn = match panel {
        PANEL_PFD => draw_pfd(&data, &ctx.pfd_cfg, &mut writer),
        PANEL_HSI => draw_hsi(&data, &mut writer),
        _ => return 0,
    };
    match drawn {
        Ok(()) => writer.finish(),
        Err(_) => 0,
    }
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

/// Renders panel `panel` (0 = PFD, 1 = HSI) from the current state block;
/// returns the scene byte length, or 0 on any failure.
#[allow(unsafe_code)] // SAFETY: as above — export attribute only, no unsafe operations.
#[unsafe(no_mangle)]
pub extern "C" fn render(panel: u32) -> u32 {
    let Ok(mut guard) = CTX.lock() else {
        return 0;
    };
    let Some(ctx) = guard.as_mut() else {
        return 0;
    };
    render_into(ctx, panel) as u32
}

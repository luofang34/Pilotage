//! Explicit WASM resource ownership and the pure instrument runtime.
//!
//! [`InstrumentRuntime`] is a JavaScript-owned wasm-bindgen resource. Each
//! resource owns its buffers, configuration, and generations; this module has
//! no process-global or thread-local mutable state.

use pilotage_alerts::{
    AlertCondition, AlertContext, AlertEvent, AlertManager, AlertOutput, AlertProfile, AltFault,
    DynFault, ManagerHealth, NavFault,
};
use pilotage_instrument_panels::{PfdConfig, VSpeeds, draw_hsi, draw_pfd};
use pilotage_instrument_scene::{LayerError, LayerId, SceneError, SceneWriter, validate_layers};
use pilotage_instrument_state::FreshnessPolicy;
use pilotage_instrument_state::abi::{AbiError, STATE_ABI_SIZE, STATE_ABI_VERSION, decode_state};
use pilotage_instrument_state::{NavSource, SignalStatus};
use wasm_bindgen::prelude::wasm_bindgen;

use crate::render_status::RenderStatus;

pub(crate) const SCENE_CAPACITY: usize = 64 * 1024;
const PACKED_SCENE_LEN_MAX: usize = 0x00ff_ffff;
const PANEL_PFD: u32 = 0;
const PANEL_HSI: u32 = 1;
const PANEL_COUNT: usize = 2;

const fn layer_bit(layer: LayerId) -> u8 {
    1u8 << layer.to_u8()
}

const PFD_CRITICAL_LAYERS: u8 =
    layer_bit(LayerId::Attitude) | layer_bit(LayerId::Tapes) | layer_bit(LayerId::Annunciation);
const HSI_CRITICAL_LAYERS: u8 = layer_bit(LayerId::Attitude)
    | layer_bit(LayerId::Tapes)
    | layer_bit(LayerId::Guidance)
    | layer_bit(LayerId::Annunciation);

pub(crate) struct Runtime {
    pub(crate) state: Vec<u8>,
    pub(crate) scene: Vec<u8>,
    pub(crate) generation: [u32; PANEL_COUNT],
    pub(crate) pfd_cfg: PfdConfig,
    /// Unusual-attitude hysteresis latches, carried across frames so
    /// tier entry/exit cannot chatter (ATT-01). Stepping twice per frame
    /// (PFD then HSI) is idempotent: the latches depend on the input
    /// magnitudes, not on step count.
    pub(crate) unusual: pilotage_instrument_state::UnusualAttitudeState,
    /// Display thresholds; the simulator profile's numbers are benchmark
    /// data, not an aircraft approval.
    pub(crate) profile: pilotage_instrument_state::AirframeDisplayProfile,
    /// The single alert state machine (ALR-01). Stepped once per
    /// [`InstrumentRuntime::step_alerts`] call; every panel render then
    /// consumes the one cached [`AlertOutput`], so the PFD and HSI can
    /// never disagree on the semantic alert state within a frame.
    pub(crate) alerts: AlertManager,
    /// Alerting profile; simulator benchmark data, not an approval.
    pub(crate) alert_profile: AlertProfile,
    /// The last stepped output. `None` until the backend steps alerts —
    /// panels then draw no alert stack, while primary-data flags render
    /// unconditionally from resolved state.
    pub(crate) alert_output: Option<AlertOutput>,
}

impl Runtime {
    fn new() -> Self {
        Self {
            state: vec![0u8; STATE_ABI_SIZE],
            scene: vec![0u8; SCENE_CAPACITY],
            generation: [0; PANEL_COUNT],
            pfd_cfg: PfdConfig::default(),
            unusual: pilotage_instrument_state::UnusualAttitudeState::default(),
            profile: pilotage_instrument_state::AirframeDisplayProfile::simulator(),
            alerts: AlertManager::new(),
            alert_profile: AlertProfile::simulator(),
            alert_output: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RenderAttempt {
    pub(crate) status: RenderStatus,
    pub(crate) scene_len: usize,
    pub(crate) generation: u32,
}

impl RenderAttempt {
    const fn failure(status: RenderStatus, generation: u32) -> Self {
        Self {
            status,
            scene_len: 0,
            generation,
        }
    }

    const fn success(scene_len: usize, generation: u32) -> Self {
        Self {
            status: RenderStatus::Ok,
            scene_len,
            generation,
        }
    }

    const fn packed(self) -> u64 {
        let scene_len = (self.scene_len as u64) & 0x00ff_ffff;
        (self.status as u64) | (scene_len << 8) | ((self.generation as u64) << 32)
    }
}

pub(crate) fn scene_error_status(error: SceneError) -> RenderStatus {
    match error {
        SceneError::BufferFull => RenderStatus::SceneBufferFull,
        SceneError::TooManyPoints | SceneError::TextTooLong => RenderStatus::SceneCommandLimit,
    }
}

fn panel_generation(runtime: &Runtime, panel_idx: usize) -> u32 {
    runtime.generation.get(panel_idx).copied().unwrap_or(0)
}

fn validate_panel_scene(panel_idx: usize, scene: &[u8]) -> RenderStatus {
    let required = match panel_idx {
        0 => PFD_CRITICAL_LAYERS,
        1 => HSI_CRITICAL_LAYERS,
        _ => return RenderStatus::InvalidPanel,
    };
    let report = match validate_layers(scene) {
        Ok(report) => report,
        Err(LayerError::Decode(_)) => return RenderStatus::SceneStructure,
        Err(_) => return RenderStatus::SceneLayerContract,
    };
    if report.present & required != required {
        return RenderStatus::SceneCriticalLayersMissing;
    }
    RenderStatus::Ok
}

/// Commits a generated scene only after the complete panel-layer contract has
/// validated. Buffer bytes are scratch until this returns success.
pub(crate) fn validate_and_commit_scene(
    runtime: &mut Runtime,
    panel_idx: usize,
    len: usize,
) -> RenderAttempt {
    let generation = panel_generation(runtime, panel_idx);
    if len > PACKED_SCENE_LEN_MAX {
        return RenderAttempt::failure(RenderStatus::SceneBufferFull, generation);
    }
    let Some(scene) = runtime.scene.get(..len) else {
        return RenderAttempt::failure(RenderStatus::SceneStructure, generation);
    };
    let status = validate_panel_scene(panel_idx, scene);
    if status != RenderStatus::Ok {
        return RenderAttempt::failure(status, generation);
    }
    let Some(next_generation) = runtime.generation.get_mut(panel_idx) else {
        return RenderAttempt::failure(RenderStatus::InvalidPanel, 0);
    };
    *next_generation = next_generation.wrapping_add(1);
    RenderAttempt::success(len, *next_generation)
}

pub(crate) fn render_into(runtime: &mut Runtime, panel: u32) -> RenderAttempt {
    let panel_idx = match panel {
        PANEL_PFD => 0usize,
        PANEL_HSI => 1,
        _ => return RenderAttempt::failure(RenderStatus::InvalidPanel, 0),
    };
    let generation = panel_generation(runtime, panel_idx);
    let state = match decode_state(&runtime.state) {
        Ok(state) => state,
        Err(AbiError::Truncated) => {
            return RenderAttempt::failure(RenderStatus::StateTruncated, generation);
        }
        Err(AbiError::BadVersion { .. }) => {
            return RenderAttempt::failure(RenderStatus::StateBadVersion, generation);
        }
    };
    let data = pilotage_instrument_state::resolve_stateful(
        &state,
        &FreshnessPolicy::default(),
        &runtime.profile,
        &mut runtime.unusual,
    );
    let mut writer = match SceneWriter::new(&mut runtime.scene) {
        Ok(writer) => writer,
        Err(error) => return RenderAttempt::failure(scene_error_status(error), generation),
    };
    let alerts = runtime.alert_output.as_ref();
    let drawn = if panel == PANEL_PFD {
        draw_pfd(&data, &runtime.pfd_cfg, alerts, &mut writer)
    } else {
        draw_hsi(&data, alerts, &mut writer)
    };
    let len = match drawn {
        Ok(()) => writer.finish(),
        Err(error) => return RenderAttempt::failure(scene_error_status(error), generation),
    };
    validate_and_commit_scene(runtime, panel_idx, len)
}

/// Maps resolved panel signals to the typed alert conditions this
/// runtime can honestly assert today: altitude, navigation, and
/// turn-rate source loss. Attitude and airspeed loss stay primary-data
/// flags only (red X), deliberately outside the alerting path, so the
/// display of a lost primary never depends on the manager. Every
/// condition is asserted or cleared each step; both operations are
/// idempotent in the manager.
fn derive_alert_events(data: &pilotage_instrument_state::PanelData) -> [AlertEvent; 3] {
    let cond = |active: bool, c: AlertCondition| {
        if active {
            AlertEvent::Assert(c)
        } else {
            AlertEvent::Clear(c)
        }
    };
    [
        cond(
            data.altitude.value_ft.status == SignalStatus::Failed,
            AlertCondition::Altitude(AltFault::Unavailable),
        ),
        cond(
            data.nav.data.source != NavSource::None && data.nav.status == SignalStatus::Failed,
            AlertCondition::Heading(NavFault::Unavailable),
        ),
        cond(
            data.turn_rate_rps.status == SignalStatus::Failed,
            AlertCondition::TurnSlip(DynFault::TurnRateInvalid),
        ),
    ]
}

/// The state-block ABI version this module was built against.
#[wasm_bindgen]
pub fn abi_version() -> u32 {
    STATE_ABI_VERSION
}

/// One explicitly owned instrument renderer and its fixed-capacity buffers.
///
/// Construction does not allocate buffers. Call [`InstrumentRuntime::init`]
/// before querying pointers or rendering; calling it again replaces all
/// buffers, configuration, and generations and invalidates earlier pointers.
#[wasm_bindgen]
#[derive(Default)]
pub struct InstrumentRuntime {
    pub(crate) runtime: Option<Runtime>,
}

#[wasm_bindgen]
impl InstrumentRuntime {
    /// Creates an uninitialized resource with no buffers.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates or replaces this resource's runtime; returns 1 on success.
    pub fn init(&mut self) -> u32 {
        self.runtime = Some(Runtime::new());
        1
    }

    /// Linear-memory offset of the packed state block, or zero before init.
    pub fn state_ptr(&self) -> u32 {
        self.runtime
            .as_ref()
            .map_or(0, |runtime| runtime.state.as_ptr() as u32)
    }

    /// Size of the packed state block in bytes.
    pub fn state_len(&self) -> u32 {
        STATE_ABI_SIZE as u32
    }

    /// Linear-memory offset of the encoded-scene buffer, or zero before init.
    pub fn scene_ptr(&self) -> u32 {
        self.runtime
            .as_ref()
            .map_or(0, |runtime| runtime.scene.as_ptr() as u32)
    }

    /// Configures speed-tape bands in knots; pass all zeros to clear.
    /// Returns a stable [`RenderStatus`] code.
    pub fn set_v_speeds(&mut self, vs0: f32, vs: f32, vfe: f32, vno: f32, vne: f32) -> u32 {
        let Some(runtime) = self.runtime.as_mut() else {
            return RenderStatus::NotInitialized as u32;
        };
        runtime.pfd_cfg.v_speeds = if vne > 0.0 {
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
        RenderStatus::Ok as u32
    }

    /// Renders a panel and returns status in bits 0..7, scene length in bits
    /// 8..31, and successful generation in bits 32..63. Failure has a zero
    /// scene length and never advances generation.
    ///
    /// Successful scene bytes remain valid until this resource's next render
    /// or init call; failure leaves scratch bytes unspecified.
    pub fn render_result(&mut self, panel: u32) -> u64 {
        self.runtime.as_mut().map_or_else(
            || RenderAttempt::failure(RenderStatus::NotInitialized, 0).packed(),
            |runtime| render_into(runtime, panel).packed(),
        )
    }

    /// Steps the alert manager once against the current state block and
    /// caches the output every subsequent panel render consumes, so all
    /// panels in a frame share one semantic alert state. `now_ms` is the
    /// caller's monotonic clock (the manager never reads an interior
    /// clock); `path_healthy == 0` marks the independent display/alerting
    /// path monitor faulted, which flags the output untrusted without
    /// suppressing it.
    ///
    /// Returns status in bits 0..7, active-alert count in bits 8..15,
    /// faulted health in bit 16, overflow in bit 17, and the manager
    /// generation in bits 32..63.
    pub fn step_alerts(&mut self, now_ms: u64, path_healthy: u32) -> u64 {
        let Some(runtime) = self.runtime.as_mut() else {
            return RenderStatus::NotInitialized as u64;
        };
        let state = match decode_state(&runtime.state) {
            Ok(state) => state,
            Err(AbiError::Truncated) => return RenderStatus::StateTruncated as u64,
            Err(AbiError::BadVersion { .. }) => return RenderStatus::StateBadVersion as u64,
        };
        let data = pilotage_instrument_state::resolve_stateful(
            &state,
            &FreshnessPolicy::default(),
            &runtime.profile,
            &mut runtime.unusual,
        );
        let events = derive_alert_events(&data);
        let ctx = AlertContext {
            declutter: data.presentation.unusual,
            alerting_path_healthy: path_healthy != 0,
            ..AlertContext::default()
        };
        let out = runtime
            .alerts
            .step(&runtime.alert_profile, &events, ctx, now_ms);
        let summary = (RenderStatus::Ok as u64)
            | ((out.active().len() as u64 & 0xff) << 8)
            | (u64::from(out.health() == ManagerHealth::Faulted) << 16)
            | (u64::from(out.overflow()) << 17)
            | ((out.generation() as u64) << 32);
        runtime.alert_output = Some(out);
        summary
    }

    /// The controlled glyph pack's canonical serialization (REN-02), for
    /// the backend's independent hash verification and atlas
    /// construction. A serialization failure returns an empty buffer,
    /// which no verifier accepts.
    pub fn glyph_manifest(&self) -> Vec<u8> {
        let manifest = pilotage_instrument_glyphs::PANEL_GLYPHS;
        let mut out = vec![0u8; manifest.canonical_len()];
        match manifest.write_canonical(&mut out) {
            Ok(len) => {
                out.truncate(len);
                out
            }
            Err(_) => Vec::new(),
        }
    }

    /// The compile-time-recorded glyph content hash the backend must
    /// match against both the canonical bytes and its own pinned value.
    pub fn glyph_recorded_hash(&self) -> Vec<u8> {
        pilotage_instrument_glyphs::PANEL_GLYPHS
            .recorded_hash()
            .to_vec()
    }
}

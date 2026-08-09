//! Explicit WASM resource ownership and the pure instrument runtime.
//!
//! [`InstrumentRuntime`] is a JavaScript-owned wasm-bindgen resource. Each
//! resource owns its buffers, configuration, and generations; this module has
//! no process-global or thread-local mutable state.

use indicate_alerts::{
    AlertCondition, AlertContext, AlertEvent, AlertManager, AlertOutput, AlertProfile, AltFault,
    DynFault, ManagerHealth, NavFault,
};
use indicate_instrument_registry::{ConfigBlob, PanelDrawError};
use indicate_instrument_scene::{LayerError, SceneError, SceneWriter, validate_layers};
use indicate_instrument_state::FreshnessPolicy;
use indicate_instrument_state::abi::v7::{self, AbiError};
use indicate_instrument_state::{NavSource, SignalStatus};
use wasm_bindgen::prelude::wasm_bindgen;

use crate::panel_registry::{canonical_frame, descriptor, registry, splice_v_speeds};
use crate::render_status::RenderStatus;

pub(crate) const SCENE_CAPACITY: usize = 64 * 1024;
const PACKED_SCENE_LEN_MAX: usize = 0x00ff_ffff;

pub(crate) struct Runtime {
    pub(crate) state: Vec<u8>,
    pub(crate) scene: Vec<u8>,
    pub(crate) generation: Vec<u32>,
    /// Per-panel validated configuration blobs, indexed like the
    /// registry composition; empty means the panel's defaults.
    pub(crate) config: Vec<Vec<u8>>,
    /// Frames that carried group tags this build cannot place, and
    /// known groups whose payloads grew — forward-compatibility
    /// telemetry the script surfaces beside unknownOpcodes.
    pub(crate) unknown_groups: u32,
    pub(crate) extended_groups: u32,
    /// Unusual-attitude hysteresis latches, carried across frames so
    /// tier entry/exit cannot chatter (ATT-01). Stepping twice per frame
    /// (PFD then HSI) is idempotent: the latches depend on the input
    /// magnitudes, not on step count.
    pub(crate) unusual: indicate_instrument_state::UnusualAttitudeState,
    /// Display thresholds; the simulator profile's numbers are benchmark
    /// data, not an aircraft approval.
    pub(crate) profile: indicate_instrument_state::AirframeDisplayProfile,
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
        let panels = registry().map_or(0, |registry| registry.panels().count());
        Self {
            state: vec![0u8; v7::CAPACITY],
            scene: vec![0u8; SCENE_CAPACITY],
            generation: vec![0; panels],
            config: vec![Vec::new(); panels],
            unknown_groups: 0,
            extended_groups: 0,
            unusual: indicate_instrument_state::UnusualAttitudeState::default(),
            profile: indicate_instrument_state::AirframeDisplayProfile::simulator(),
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
    // The critical-layer masks are owned by the panel descriptors
    // (ADR-0029): this shell holds no mask or panel list of its own.
    let Some(required) = descriptor(panel_idx as u32).map(|d| d.required_layers) else {
        return RenderStatus::InvalidPanel;
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
    let Some(panel_descriptor) = descriptor(panel) else {
        return RenderAttempt::failure(RenderStatus::InvalidPanel, 0);
    };
    let panel_idx = panel as usize;
    let generation = panel_generation(runtime, panel_idx);
    let state = match v7::decode_state(&runtime.state) {
        Ok(report) => report.state,
        Err(AbiError::Truncated) => {
            return RenderAttempt::failure(RenderStatus::StateTruncated, generation);
        }
        Err(AbiError::BadVersion { .. }) => {
            return RenderAttempt::failure(RenderStatus::StateBadVersion, generation);
        }
        Err(AbiError::NonCanonicalOrder { .. } | AbiError::GroupTruncated { .. }) => {
            return RenderAttempt::failure(RenderStatus::StateMalformed, generation);
        }
    };
    let data = indicate_instrument_state::resolve_stateful(
        &state,
        &FreshnessPolicy::default(),
        &runtime.profile,
        &mut runtime.unusual,
    );
    let config_bytes: &[u8] = runtime.config.get(panel_idx).map_or(&[], |bytes| bytes);
    let Ok(config) = ConfigBlob::parse(config_bytes) else {
        return RenderAttempt::failure(RenderStatus::ConfigInvalid, generation);
    };
    let mut writer = match SceneWriter::new(&mut runtime.scene) {
        Ok(writer) => writer,
        Err(error) => return RenderAttempt::failure(scene_error_status(error), generation),
    };
    let alerts = runtime.alert_output.as_ref();
    // The same canonical frame the design-dimension exports publish, so
    // the frame this scene is emitted at is the one the backend maps.
    let frame = canonical_frame(panel_descriptor);
    let len = match (panel_descriptor.draw)(&data, &config, alerts, frame, &mut writer) {
        Ok(()) => writer.finish(),
        Err(PanelDrawError::Scene(error)) => {
            return RenderAttempt::failure(scene_error_status(error), generation);
        }
        Err(PanelDrawError::Config(_)) => {
            return RenderAttempt::failure(RenderStatus::ConfigInvalid, generation);
        }
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
fn derive_alert_events(data: &indicate_instrument_state::PanelData) -> [AlertEvent; 3] {
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
            data.turn.rate_rps.status == SignalStatus::Failed,
            AlertCondition::TurnSlip(DynFault::TurnRateInvalid),
        ),
    ]
}

/// The state-frame ABI version this module was built against.
#[wasm_bindgen]
pub fn abi_version() -> u32 {
    u32::from(v7::VERSION)
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

    /// Linear-memory offset of the state-frame buffer, or zero before init.
    pub fn state_ptr(&self) -> u32 {
        self.runtime
            .as_ref()
            .map_or(0, |runtime| runtime.state.as_ptr() as u32)
    }

    /// Capacity of the state-frame buffer in bytes. The v7 frame is
    /// self-delimiting, so the writer needs a bound, not an exact size;
    /// growing the capacity is not a wire break.
    pub fn state_capacity(&self) -> u32 {
        v7::CAPACITY as u32
    }

    /// Linear-memory offset of the encoded-scene buffer, or zero before init.
    pub fn scene_ptr(&self) -> u32 {
        self.runtime
            .as_ref()
            .map_or(0, |runtime| runtime.scene.as_ptr() as u32)
    }

    /// Configures speed-tape bands in knots; pass all zeros to clear.
    /// Returns a stable [`RenderStatus`] code. This is sugar over the
    /// PFD's config blob: it splices the V_SPEEDS entry and leaves
    /// every other configured key alone.
    pub fn set_v_speeds(&mut self, vs0: f32, vs: f32, vfe: f32, vno: f32, vne: f32) -> u32 {
        let Some(runtime) = self.runtime.as_mut() else {
            return RenderStatus::NotInitialized as u32;
        };
        let payload = (vne > 0.0).then(|| {
            let mut bytes = [0u8; 20];
            for (slot, value) in bytes.chunks_exact_mut(4).zip([vs0, vs, vfe, vno, vne]) {
                slot.copy_from_slice(&value.to_le_bytes());
            }
            bytes
        });
        // Resolved by descriptor id, never by position: composition is
        // the extension point, so panel 0 is not necessarily the PFD.
        let Some((index, panel)) = registry().and_then(|registry| {
            registry
                .panels()
                .enumerate()
                .find(|(_, panel)| panel.id == "pfd")
        }) else {
            return RenderStatus::InvalidPanel as u32;
        };
        let Some(current) = runtime.config.get(index) else {
            return RenderStatus::InvalidPanel as u32;
        };
        let Some(next) = splice_v_speeds(current, panel.config_schema, payload) else {
            return RenderStatus::ConfigInvalid as u32;
        };
        let gated =
            ConfigBlob::parse(&next).and_then(|parsed| parsed.require_schema(panel.config_schema));
        if gated.is_err() {
            return RenderStatus::ConfigInvalid as u32;
        }
        runtime.config[index] = next;
        RenderStatus::Ok as u32
    }

    /// Replaces a panel's configuration with `blob` (the bounded
    /// key-TLV encoding), validated for framing and against the
    /// panel's declared schema before it is accepted — a refused blob
    /// leaves the previous configuration in effect. Returns a stable
    /// [`RenderStatus`] code.
    pub fn set_panel_config(&mut self, panel: u32, blob: &[u8]) -> u32 {
        let Some(runtime) = self.runtime.as_mut() else {
            return RenderStatus::NotInitialized as u32;
        };
        let Some(panel_descriptor) = descriptor(panel) else {
            return RenderStatus::InvalidPanel as u32;
        };
        let Some(slot) = runtime.config.get_mut(panel as usize) else {
            return RenderStatus::InvalidPanel as u32;
        };
        let accepted = ConfigBlob::parse(blob)
            .and_then(|parsed| parsed.require_schema(panel_descriptor.config_schema));
        if accepted.is_err() {
            return RenderStatus::ConfigInvalid as u32;
        }
        *slot = blob.to_vec();
        RenderStatus::Ok as u32
    }

    /// Wrapping count of decoded frames' group tags this build cannot
    /// place — a producer ahead of this consumer is visible, not
    /// silent (mirrors the scene path's unknownOpcodes).
    pub fn state_unknown_groups(&self) -> u32 {
        self.runtime.as_ref().map_or(0, |r| r.unknown_groups)
    }

    /// Wrapping count of known groups whose payloads carried grown
    /// tails this build skipped.
    pub fn state_extended_groups(&self) -> u32 {
        self.runtime.as_ref().map_or(0, |r| r.extended_groups)
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
        let state = match v7::decode_state(&runtime.state) {
            Ok(report) => {
                // Counted here, once per frame step, so a frame rendered
                // across N panels does not multiply its tag counts.
                runtime.unknown_groups = runtime
                    .unknown_groups
                    .wrapping_add(u32::from(report.unknown_groups));
                runtime.extended_groups = runtime
                    .extended_groups
                    .wrapping_add(u32::from(report.extended_groups));
                report.state
            }
            Err(AbiError::Truncated) => return RenderStatus::StateTruncated as u64,
            Err(AbiError::BadVersion { .. }) => return RenderStatus::StateBadVersion as u64,
            Err(AbiError::NonCanonicalOrder { .. } | AbiError::GroupTruncated { .. }) => {
                return RenderStatus::StateMalformed as u64;
            }
        };
        let data = indicate_instrument_state::resolve_stateful(
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
        let manifest = indicate_instrument_glyphs::PANEL_GLYPHS;
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
        indicate_instrument_glyphs::PANEL_GLYPHS
            .recorded_hash()
            .to_vec()
    }
}

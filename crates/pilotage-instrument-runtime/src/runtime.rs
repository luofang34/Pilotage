//! The portable instrument runtime core: state decode and resolve, scene
//! generation and validation, successful-production generations, alert
//! step, and panel configuration (ADR-0032).
//!
//! [`Runtime`] owns its buffers, configuration, and generations; this
//! module has no process-global or thread-local mutable state. Every
//! result is a typed value: shells pack or marshal, they do not decide.

use indicate_alerts::{
    AlertCondition, AlertContext, AlertEvent, AlertManager, AlertOutput, AlertProfile, AltFault,
    DynFault, ManagerHealth, NavFault,
};
use indicate_instrument_registry::{ConfigBlob, PanelDrawError};
use indicate_instrument_scene::{LayerError, SceneError, SceneWriter, validate_layers};
use indicate_instrument_state::FreshnessPolicy;
use indicate_instrument_state::abi::v7::{self, AbiError};
use indicate_instrument_state::{NavSource, SignalStatus};

use crate::registry::{canonical_frame, descriptor, registry, splice_v_speeds};
use crate::render_status::RenderStatus;

pub(crate) const SCENE_CAPACITY: usize = 64 * 1024;
/// The largest scene length a shell's packed result can carry; the
/// commit path refuses longer scenes before a shell ever sees them.
const PACKED_SCENE_LEN_MAX: usize = 0x00ff_ffff;

/// One explicitly owned instrument renderer and its fixed-capacity
/// buffers.
///
/// Construction allocates buffers once; they never grow, so a shell can
/// hand their addresses to its consumer and they stay valid until the
/// resource is dropped or replaced.
pub struct Runtime {
    pub(crate) state: Vec<u8>,
    pub(crate) scene: Vec<u8>,
    pub(crate) generation: Vec<u32>,
    pub(crate) composition_scene: Vec<u8>,
    pub(crate) composition_panels: Vec<crate::CompositionPanelOutcome>,
    pub(crate) composition_generation: u32,
    /// Per-panel validated configuration blobs, indexed like the
    /// registry composition; empty means the panel's defaults.
    pub(crate) config: Vec<Vec<u8>>,
    /// Frames that carried group tags this build cannot place, and
    /// known groups whose payloads grew — forward-compatibility
    /// telemetry a shell surfaces beside its unknown-opcode count.
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
    /// [`Runtime::step_alerts`] call; every panel render then consumes
    /// the one cached [`AlertOutput`], so the PFD and HSI can never
    /// disagree on the semantic alert state within a frame.
    pub(crate) alerts: AlertManager,
    /// Alerting profile; simulator benchmark data, not an approval.
    pub(crate) alert_profile: AlertProfile,
    /// The last stepped output. `None` until the shell steps alerts —
    /// panels then draw no alert stack, while primary-data flags render
    /// unconditionally from resolved state.
    pub(crate) alert_output: Option<AlertOutput>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    /// Allocates a runtime with zeroed buffers, default configuration,
    /// and one generation counter per composed panel.
    pub fn new() -> Self {
        let panels = registry().map_or(0, |registry| registry.panels().count());
        let slots = crate::composition_slot_count() as usize;
        Self {
            state: vec![0u8; v7::CAPACITY],
            scene: vec![0u8; SCENE_CAPACITY],
            generation: vec![0; panels],
            composition_scene: vec![0; SCENE_CAPACITY.saturating_mul(slots)],
            composition_panels: vec![crate::CompositionPanelOutcome::empty(); slots],
            composition_generation: 0,
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

    /// The state-frame buffer a producer writes each frame.
    pub fn state(&self) -> &[u8] {
        &self.state
    }

    /// The state-frame buffer, mutable so a shell's consumer can write
    /// the next frame in place.
    pub fn state_mut(&mut self) -> &mut [u8] {
        &mut self.state
    }

    /// Capacity of the state-frame buffer in bytes. The v7 frame is
    /// self-delimiting, so the writer needs a bound, not an exact size;
    /// growing the capacity is not a wire break.
    pub const fn state_capacity() -> usize {
        v7::CAPACITY
    }

    /// The encoded-scene scratch buffer. Its committed prefix is the
    /// last successful render's [`RenderOutcome::scene_len`] bytes; the
    /// rest is scratch and unspecified.
    pub fn scene(&self) -> &[u8] {
        &self.scene
    }

    /// Wrapping count of decoded frames' group tags this build cannot
    /// place — a producer ahead of this consumer is visible, not silent.
    pub fn unknown_groups(&self) -> u32 {
        self.unknown_groups
    }

    /// Wrapping count of known groups whose payloads carried grown
    /// tails this build skipped.
    pub fn extended_groups(&self) -> u32 {
        self.extended_groups
    }

    pub(crate) fn panel_generation(&self, panel_idx: usize) -> u32 {
        self.generation.get(panel_idx).copied().unwrap_or(0)
    }

    /// Commits a generated scene only after the complete panel-layer
    /// contract has validated. Buffer bytes are scratch until this
    /// returns success.
    pub fn validate_and_commit_scene(&mut self, panel_idx: usize, len: usize) -> RenderOutcome {
        let generation = self.panel_generation(panel_idx);
        if len > PACKED_SCENE_LEN_MAX {
            return RenderOutcome::failure(RenderStatus::SceneBufferFull, generation);
        }
        let Some(scene) = self.scene.get(..len) else {
            return RenderOutcome::failure(RenderStatus::SceneStructure, generation);
        };
        let status = validate_panel_scene(panel_idx, scene);
        if status != RenderStatus::Ok {
            return RenderOutcome::failure(status, generation);
        }
        let Some(next_generation) = self.generation.get_mut(panel_idx) else {
            return RenderOutcome::failure(RenderStatus::InvalidPanel, 0);
        };
        *next_generation = next_generation.wrapping_add(1);
        RenderOutcome::success(len as u32, *next_generation)
    }

    /// Renders a panel from the current state block. Failure carries a
    /// zero scene length and never advances generation; successful scene
    /// bytes remain valid until the next render call.
    pub fn render(&mut self, panel: u32) -> RenderOutcome {
        let Some(panel_descriptor) = descriptor(panel) else {
            return RenderOutcome::failure(RenderStatus::InvalidPanel, 0);
        };
        let panel_idx = panel as usize;
        let generation = self.panel_generation(panel_idx);
        let state = match v7::decode_state(&self.state) {
            Ok(report) => report.state,
            Err(AbiError::Truncated) => {
                return RenderOutcome::failure(RenderStatus::StateTruncated, generation);
            }
            Err(AbiError::BadVersion { .. }) => {
                return RenderOutcome::failure(RenderStatus::StateBadVersion, generation);
            }
            Err(AbiError::NonCanonicalOrder { .. } | AbiError::GroupTruncated { .. }) => {
                return RenderOutcome::failure(RenderStatus::StateMalformed, generation);
            }
        };
        let data = indicate_instrument_state::resolve_stateful(
            &state,
            &FreshnessPolicy::default(),
            &self.profile,
            &mut self.unusual,
        );
        let config_bytes: &[u8] = self.config.get(panel_idx).map_or(&[], |bytes| bytes);
        let Ok(config) = ConfigBlob::parse(config_bytes) else {
            return RenderOutcome::failure(RenderStatus::ConfigInvalid, generation);
        };
        let mut writer = match SceneWriter::new(&mut self.scene) {
            Ok(writer) => writer,
            Err(error) => return RenderOutcome::failure(scene_error_status(error), generation),
        };
        let alerts = self.alert_output.as_ref();
        // The same canonical frame the design-dimension helpers publish,
        // so the frame this scene is emitted at is the one the shell maps.
        let frame = canonical_frame(panel_descriptor);
        let len = match (panel_descriptor.draw)(&data, &config, alerts, frame, &mut writer) {
            Ok(()) => writer.finish(),
            Err(PanelDrawError::Scene(error)) => {
                return RenderOutcome::failure(scene_error_status(error), generation);
            }
            Err(PanelDrawError::Config(_)) => {
                return RenderOutcome::failure(RenderStatus::ConfigInvalid, generation);
            }
        };
        self.validate_and_commit_scene(panel_idx, len)
    }

    /// Steps the alert manager once against the current state block and
    /// caches the output every subsequent panel render consumes, so all
    /// panels in a frame share one semantic alert state. `now_ms` is the
    /// caller's monotonic clock (the manager never reads an interior
    /// clock); `path_healthy == false` marks the independent
    /// display/alerting path monitor faulted, which flags the output
    /// untrusted without suppressing it.
    pub fn step_alerts(&mut self, now_ms: u64, path_healthy: bool) -> AlertStepOutcome {
        let state = match v7::decode_state(&self.state) {
            Ok(report) => {
                // Counted here, once per frame step, so a frame rendered
                // across N panels does not multiply its tag counts.
                self.unknown_groups = self
                    .unknown_groups
                    .wrapping_add(u32::from(report.unknown_groups));
                self.extended_groups = self
                    .extended_groups
                    .wrapping_add(u32::from(report.extended_groups));
                report.state
            }
            Err(AbiError::Truncated) => {
                return AlertStepOutcome::failure(RenderStatus::StateTruncated);
            }
            Err(AbiError::BadVersion { .. }) => {
                return AlertStepOutcome::failure(RenderStatus::StateBadVersion);
            }
            Err(AbiError::NonCanonicalOrder { .. } | AbiError::GroupTruncated { .. }) => {
                return AlertStepOutcome::failure(RenderStatus::StateMalformed);
            }
        };
        let data = indicate_instrument_state::resolve_stateful(
            &state,
            &FreshnessPolicy::default(),
            &self.profile,
            &mut self.unusual,
        );
        let events = derive_alert_events(&data);
        let ctx = AlertContext {
            declutter: data.presentation.unusual,
            alerting_path_healthy: path_healthy,
            ..AlertContext::default()
        };
        let out = self.alerts.step(&self.alert_profile, &events, ctx, now_ms);
        let outcome = AlertStepOutcome {
            status: RenderStatus::Ok,
            active_count: out.active().len() as u32,
            faulted: out.health() == ManagerHealth::Faulted,
            overflow: out.overflow(),
            manager_generation: out.generation(),
        };
        self.alert_output = Some(out);
        outcome
    }

    /// Configures speed-tape bands in knots; pass all zeros to clear.
    /// This is sugar over the PFD's config blob: it splices the
    /// V_SPEEDS entry and leaves every other configured key alone.
    pub fn set_v_speeds(
        &mut self,
        vs0: f32,
        vs: f32,
        vfe: f32,
        vno: f32,
        vne: f32,
    ) -> RenderStatus {
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
            return RenderStatus::InvalidPanel;
        };
        let Some(current) = self.config.get(index) else {
            return RenderStatus::InvalidPanel;
        };
        let Some(next) = splice_v_speeds(current, panel.config_schema, payload) else {
            return RenderStatus::ConfigInvalid;
        };
        let gated =
            ConfigBlob::parse(&next).and_then(|parsed| parsed.require_schema(panel.config_schema));
        if gated.is_err() {
            return RenderStatus::ConfigInvalid;
        }
        self.config[index] = next;
        RenderStatus::Ok
    }

    /// Replaces a panel's configuration with `blob` (the bounded
    /// key-TLV encoding), validated for framing and against the
    /// panel's declared schema before it is accepted — a refused blob
    /// leaves the previous configuration in effect.
    pub fn set_panel_config(&mut self, panel: u32, blob: &[u8]) -> RenderStatus {
        let Some(panel_descriptor) = descriptor(panel) else {
            return RenderStatus::InvalidPanel;
        };
        let Some(slot) = self.config.get_mut(panel as usize) else {
            return RenderStatus::InvalidPanel;
        };
        let accepted = ConfigBlob::parse(blob)
            .and_then(|parsed| parsed.require_schema(panel_descriptor.config_schema));
        if accepted.is_err() {
            return RenderStatus::ConfigInvalid;
        }
        *slot = blob.to_vec();
        RenderStatus::Ok
    }
}

/// The typed result of one render attempt: the producer status, the
/// committed scene length in bytes (zero on failure), and the panel's
/// successful-production generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOutcome {
    /// Why the attempt succeeded or failed.
    pub status: RenderStatus,
    /// Committed scene length in bytes; zero on every failure.
    pub scene_len: u32,
    /// The panel's successful-production generation. Advances only on
    /// success, so it is a liveness signal no failed attempt can fake.
    pub generation: u32,
}

impl RenderOutcome {
    /// A failed attempt: zero length, generation unchanged.
    pub const fn failure(status: RenderStatus, generation: u32) -> Self {
        Self {
            status,
            scene_len: 0,
            generation,
        }
    }

    /// A successful production of `scene_len` bytes.
    pub const fn success(scene_len: u32, generation: u32) -> Self {
        Self {
            status: RenderStatus::Ok,
            scene_len,
            generation,
        }
    }
}

/// The typed result of one alert-manager step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertStepOutcome {
    /// Why the step succeeded or failed. A decode failure carries zero
    /// in every other field.
    pub status: RenderStatus,
    /// Number of active alerts after the step.
    pub active_count: u32,
    /// The independent display/alerting path monitor is faulted; the
    /// output is flagged untrusted, not suppressed.
    pub faulted: bool,
    /// The manager dropped events this step.
    pub overflow: bool,
    /// The manager's own generation.
    pub manager_generation: u32,
}

impl AlertStepOutcome {
    /// A failed step: status set, every other field zero.
    pub const fn failure(status: RenderStatus) -> Self {
        Self {
            status,
            active_count: 0,
            faulted: false,
            overflow: false,
            manager_generation: 0,
        }
    }
}

/// Maps a scene-encoding error to its producer status. Buffer
/// exhaustion and per-command limits are different operator diagnoses
/// (capacity budget vs panel defect) and must not collapse.
pub fn scene_error_status(error: SceneError) -> RenderStatus {
    match error {
        SceneError::BufferFull => RenderStatus::SceneBufferFull,
        SceneError::TooManyPoints | SceneError::TextTooLong => RenderStatus::SceneCommandLimit,
    }
}

pub(crate) fn validate_panel_scene(panel_idx: usize, scene: &[u8]) -> RenderStatus {
    // The critical-layer masks are owned by the panel descriptors
    // (ADR-0029): this runtime holds no mask or panel list of its own.
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

/// Maps resolved panel signals to the typed alert conditions this
/// runtime can honestly assert today: altitude, navigation, and
/// turn-rate source loss. Attitude and airspeed loss stay primary-data
/// flags only (red X), deliberately outside the alerting path, so the
/// display of a lost primary never depends on the manager. Every
/// condition is asserted or cleared each step; both operations are
/// idempotent in the manager.
pub fn derive_alert_events(data: &indicate_instrument_state::PanelData) -> [AlertEvent; 3] {
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

/// The state-frame ABI version this crate was built against.
pub fn abi_version() -> u32 {
    u32::from(v7::VERSION)
}

/// The scene format version this crate was built against.
pub fn scene_format_version() -> u32 {
    u32::from(indicate_instrument_scene::SCENE_FORMAT_VERSION)
}

/// The controlled glyph pack's canonical serialization (REN-02), for a
/// shell's independent hash verification and atlas construction. A
/// serialization failure returns an empty buffer, which no verifier
/// accepts.
pub fn glyph_manifest() -> Vec<u8> {
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

/// The compile-time-recorded glyph content hash a shell must match
/// against both the canonical bytes and its own pinned value.
pub fn glyph_recorded_hash() -> Vec<u8> {
    indicate_instrument_glyphs::PANEL_GLYPHS
        .recorded_hash()
        .to_vec()
}

//! The wasm-bindgen surface: a JS-owned control resource with reusable input
//! and output buffers, so one `evaluate` call per tick touches no allocation,
//! JSON, hashing, or network. The browser writes the raw gamepad sample into
//! the input buffer, calls `evaluate`, and reads the plan out of the output
//! buffer — that is the entire hot path.

use wasm_bindgen::prelude::wasm_bindgen;

use crate::coordinator::ControlCoordinator;
use crate::device::SelectOutcome;
use crate::plan::{ControlPlan, LeaseAction};
use crate::profile::DEFAULT_PROFILE_BYTES;
use crate::sample::{ButtonSample, Mode, RawSample, SessionState};

use pilotage_input::ProfileLayer;

/// The built-in default profile bytes, so the shell can bootstrap through the
/// SAME `activate` path any other source uses — never a privileged default
/// call. The bytes are the single `include_bytes!` source; the shell just
/// hands them back into [`WebControl::activate`].
#[wasm_bindgen]
#[must_use]
pub fn default_profile() -> Vec<u8> {
    DEFAULT_PROFILE_BYTES.to_vec()
}

use crate::device::{MAX_AXES, MAX_BUTTONS};

/// Raw-sample source selector for [`WebControl::evaluate`]: `0` reads the
/// input buffer as a pad sample routed through the selected device profile;
/// `1` ignores the buffer and synthesizes from held keys via the keyboard
/// profile.
const SOURCE_KEYS: u32 = 1;
// Input layout: axes f32[8] | values f32[24] | pressed-bitset u32.
const IN_AXES: usize = 0;
const IN_VALUES: usize = IN_AXES + MAX_AXES * 4;
const IN_PRESSED: usize = IN_VALUES + MAX_BUTTONS * 4;
const IN_LEN: usize = IN_PRESSED + 4;
// Output layout: flags u32 | motion f32[4] | gimbal f32[2].
const OUT_FLAGS: usize = 0;
const OUT_MOTION: usize = 4;
const OUT_GIMBAL: usize = OUT_MOTION + 4 * 4;
const OUT_LEN: usize = OUT_GIMBAL + 2 * 4;

// evaluate() return flags.
const FLAG_MOTION: u32 = 1;
const FLAG_GIMBAL: u32 = 1 << 1;
const FLAG_RECENTER: u32 = 1 << 2;
const FLAG_ARM: u32 = 1 << 3;
const FLAG_DISARM: u32 = 1 << 4;
const FLAG_CAPTURE: u32 = 1 << 5;
const LEASE_SHIFT: u32 = 8; // bits 8..9: gimbal lease 0 none, 1 request, 2 release.
const MOTION_LEASE_SHIFT: u32 = 10; // bits 10..11: motion lease, same encoding.

/// The JS-owned web-control resource. Construct it, write the built-in default
/// profile bytes through [`WebControl::activate`], then drive ticks through
/// [`WebControl::evaluate`].
#[wasm_bindgen]
pub struct WebControl {
    coordinator: ControlCoordinator,
    sample: RawSample,
    input: Vec<u8>,
    output: Vec<u8>,
}

#[wasm_bindgen]
impl WebControl {
    /// A resource with an empty runtime and its fixed-capacity buffers.
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self {
            coordinator: ControlCoordinator::new(),
            sample: RawSample::default(),
            input: vec![0u8; IN_LEN],
            output: vec![0u8; OUT_LEN],
        }
    }

    /// Linear-memory offset of the input buffer the caller writes each tick.
    #[must_use]
    pub fn input_ptr(&self) -> u32 {
        self.input.as_ptr() as u32
    }

    /// Linear-memory offset of the output buffer the caller reads each tick.
    #[must_use]
    pub fn output_ptr(&self) -> u32 {
        self.output.as_ptr() as u32
    }

    /// Compiles and activates candidate profile bytes through the same
    /// validated seam any source uses. Returns the activation revision on
    /// success, or `0` if the candidate failed to compile (the previously
    /// active profile, if any, stays active).
    pub fn activate(&mut self, candidate: &[u8]) -> u32 {
        self.coordinator.activate_scheme(candidate)
    }

    /// The current session activation revision.
    #[must_use]
    pub fn activation_revision(&self) -> u32 {
        self.coordinator.activation_revision()
    }

    /// Re-opens the activation transaction for the current mapping (neutral
    /// handover, gimbal + motion lease cycle, revision advance on install)
    /// without changing it — the shell's seam for a motion-scope handover.
    /// Returns false before the first activation.
    pub fn reactivate(&mut self) -> bool {
        self.coordinator.reactivate()
    }

    /// Evaluates one control tick from the input buffer and the session
    /// scalars, writes the plan into the output buffer, and returns the plan
    /// flags. `mode` is 0 pilot, 1 cruise, 2 fpv, 3 rover; `session` packs
    /// bit0 connected, bit1 gimbal-lease-granted, bit2 gimbal-lease-denied,
    /// bit3 motion-lease-granted, bit4 motion-lease-denied, bit5 motion-recovered.
    /// `source` is `0` for a pad sample in the input buffer, `1` for the
    /// held-key state (the input buffer is ignored).
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &mut self,
        axis_count: u32,
        button_count: u32,
        mode: u32,
        now_ms: f64,
        session: u32,
        generation: u32,
        source: u32,
    ) -> u32 {
        self.load_sample(axis_count as usize, button_count as usize, source);
        let state = SessionState {
            generation,
            now_ms,
            mode: mode_from_u32(mode),
            connected: session & 1 != 0,
            lease_granted: session & (1 << 1) != 0,
            lease_denied: session & (1 << 2) != 0,
            motion_granted: session & (1 << 3) != 0,
            motion_denied: session & (1 << 4) != 0,
            motion_recovered: session & (1 << 5) != 0,
        };
        let plan = self.coordinator.evaluate(&self.sample, &state);
        self.store_plan(&plan)
    }

    /// The active profile's identity string (empty before activation).
    #[must_use]
    pub fn profile_id(&self) -> String {
        self.coordinator.profile_id().to_owned()
    }

    /// The active profile DOCUMENT revision (ADR-0007/0009), carried on control
    /// frames as `profile_revision`. Distinct from the activation epoch.
    #[must_use]
    pub fn profile_revision(&self) -> u32 {
        self.coordinator.profile_revision()
    }

    /// The active profile's 32-byte content digest (all-zero before
    /// activation). Exposed so a host can bind the on-wire activation revision
    /// to the exact bytes that produced it.
    #[must_use]
    pub fn profile_digest(&self) -> Vec<u8> {
        self.coordinator.profile_digest().to_vec()
    }

    /// Resolves a `Gamepad.id` string through the shared layered registry.
    /// Returns `1` for an exact vendor/product match, `2` for the generic
    /// fallback, `0` when refused (an ambiguous layer fails closed:
    /// subsequent pad ticks read an empty sample and drive nothing). A
    /// selection that changes the effective mapping swaps TRANSACTIONALLY:
    /// the runtime cycles the gimbal + motion leases through a neutral
    /// handover, the map installs only when the handover completes, and the
    /// activation revision advances — so the shell re-announces and a
    /// deflected input on the new pad can never drive the old authority.
    pub fn select_device(&mut self, gamepad_id: &str) -> u32 {
        match self.coordinator.select_device(gamepad_id) {
            SelectOutcome::Refused => 0,
            SelectOutcome::Exact => 1,
            SelectOutcome::Fallback => 2,
        }
    }

    /// The selected pad is gone (disconnect, or a replacement's disconnect
    /// half): control returns to the layered keyboard profile through the
    /// SAME transactional handover a selection takes, and the announcement
    /// flips to the keyboard's real identity, revision, and digest.
    pub fn deselect_device(&mut self) {
        self.coordinator.deselect_device();
    }

    /// Adds a device profile to a registry layer (`1` organization, `2`
    /// user, `3` vehicle, `4` session; the built-in layer is fixed at
    /// build time), then re-resolves the current pad so an override takes
    /// the same transactional path a physical swap takes. Returns false
    /// when the bytes fail shared-engine validation or the layer code is
    /// unknown.
    pub fn add_device_profile(&mut self, layer: u32, bytes: &[u8]) -> bool {
        let layer = match layer {
            1 => ProfileLayer::Organization,
            2 => ProfileLayer::User,
            3 => ProfileLayer::Vehicle,
            4 => ProfileLayer::Session,
            _ => return false,
        };
        self.coordinator.add_device_profile(layer, bytes)
    }

    /// The INSTALLED pad profile's human-readable label (empty when refused
    /// or while a swap is still pending its handover). Doubles as the
    /// device profile identity in the activation announcement.
    #[must_use]
    pub fn device_label(&self) -> String {
        self.coordinator.device_label().to_owned()
    }

    /// The installed pad profile's document revision (zero when none).
    #[must_use]
    pub fn device_revision(&self) -> u32 {
        self.coordinator.device_revision()
    }

    /// The installed pad profile's effective-content digest (empty when no
    /// pad map is installed), binding the announcement to the exact merged
    /// document that routes physical input.
    #[must_use]
    pub fn device_digest(&self) -> Vec<u8> {
        self.coordinator
            .device_digest()
            .map_or_else(Vec::new, |digest| digest.to_vec())
    }

    /// Records one keyboard transition. `key` is the canonical
    /// `KeyboardEvent.key` value with single letters lower-cased — the
    /// convention the keyboard profile data speaks.
    pub fn key_event(&mut self, key: &str, pressed: bool) {
        self.coordinator.key_event(key, pressed);
    }

    /// Drops every held key (window blur or session teardown), so a key
    /// released while the page was blurred cannot remain a phantom hold.
    pub fn clear_keys(&mut self) {
        self.coordinator.clear_keys();
    }

    /// Whether the keyboard profile binds `key`, so the shell knows which
    /// keys to capture without holding any key list of its own.
    #[must_use]
    pub fn key_is_bound(&self, key: &str) -> bool {
        self.coordinator.stage().key_is_bound(key)
    }
}

impl Default for WebControl {
    fn default() -> Self {
        Self::new()
    }
}

impl WebControl {
    /// Refills the reusable [`RawSample`] for the tick: a pad source reads
    /// the raw input buffer and routes it through the selected device
    /// profile; the key source ignores the buffer and synthesizes from held
    /// keys via the keyboard profile. Fixed scratch arrays keep the steady
    /// tick allocation-free.
    fn load_sample(&mut self, axes: usize, buttons: usize, source: u32) {
        if source == SOURCE_KEYS {
            self.coordinator.key_sample(&mut self.sample);
            return;
        }
        let axes = axes.min(MAX_AXES);
        let buttons = buttons.min(MAX_BUTTONS);
        let pressed = read_u32(&self.input, IN_PRESSED);
        let mut raw_axes = [0.0f32; MAX_AXES];
        for (i, slot) in raw_axes.iter_mut().take(axes).enumerate() {
            *slot = read_f32(&self.input, IN_AXES + i * 4);
        }
        let mut raw_buttons = [ButtonSample::default(); MAX_BUTTONS];
        for (i, slot) in raw_buttons.iter_mut().take(buttons).enumerate() {
            *slot = ButtonSample {
                pressed: pressed & (1u32 << u32::try_from(i).unwrap_or(31)) != 0,
                value: read_f32(&self.input, IN_VALUES + i * 4),
            };
        }
        self.coordinator
            .pad_sample(&raw_axes[..axes], &raw_buttons[..buttons], &mut self.sample);
    }

    /// Encodes the plan into the output buffer and returns its flags.
    fn store_plan(&mut self, plan: &ControlPlan) -> u32 {
        let mut flags = 0u32;
        write_f32x(&mut self.output, OUT_MOTION, [0.0; 4]);
        write_f32x(&mut self.output, OUT_GIMBAL, [0.0; 2]);
        if let Some(motion) = &plan.motion {
            flags |= FLAG_MOTION;
            write_frame_axes(&mut self.output, OUT_MOTION, motion, 4);
        }
        // Arm/disarm are TYPED, never physical button ids, so a rebound arm
        // control cannot silently disable arming.
        if plan.arm {
            flags |= FLAG_ARM;
        }
        if plan.disarm {
            flags |= FLAG_DISARM;
        }
        if plan.capture_active {
            flags |= FLAG_CAPTURE;
        }
        if let Some(gimbal) = &plan.gimbal {
            flags |= FLAG_GIMBAL;
            write_frame_axes(&mut self.output, OUT_GIMBAL, gimbal, 2);
            if !gimbal.edges().is_empty() {
                flags |= FLAG_RECENTER;
            }
        }
        flags |= lease_bits(plan.lease, LEASE_SHIFT);
        flags |= lease_bits(plan.motion_lease, MOTION_LEASE_SHIFT);
        write_u32(&mut self.output, OUT_FLAGS, flags);
        flags
    }
}

/// Packs a lease action into two bits at `shift`: 0 none, 1 request, 2 release.
fn lease_bits(action: Option<LeaseAction>, shift: u32) -> u32 {
    match action {
        Some(LeaseAction::Request) => 1 << shift,
        Some(LeaseAction::Release) => 2 << shift,
        None => 0,
    }
}

fn mode_from_u32(mode: u32) -> Mode {
    match mode {
        1 => Mode::QuadCruise,
        2 => Mode::Fpv,
        3 => Mode::Rover,
        _ => Mode::QuadPilot,
    }
}

/// Writes the first `count` axis values of a frame into the buffer in axis-id
/// order (the runtime emits them already ordered).
fn write_frame_axes(buffer: &mut [u8], offset: usize, frame: &crate::plan::Frame, count: usize) {
    for (slot, (_, value)) in frame.axes().iter().take(count).enumerate() {
        write_f32(buffer, offset + slot * 4, *value);
    }
}

fn read_f32(buffer: &[u8], offset: usize) -> f32 {
    buffer
        .get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .map_or(0.0, f32::from_le_bytes)
}

fn read_u32(buffer: &[u8], offset: usize) -> u32 {
    buffer
        .get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

fn write_f32(buffer: &mut [u8], offset: usize, value: f32) {
    if let Some(slice) = buffer.get_mut(offset..offset + 4) {
        slice.copy_from_slice(&value.to_le_bytes());
    }
}

fn write_f32x<const N: usize>(buffer: &mut [u8], offset: usize, values: [f32; N]) {
    for (i, value) in values.into_iter().enumerate() {
        write_f32(buffer, offset + i * 4, value);
    }
}

fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    if let Some(slice) = buffer.get_mut(offset..offset + 4) {
        slice.copy_from_slice(&value.to_le_bytes());
    }
}

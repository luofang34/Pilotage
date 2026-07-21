//! The browser device stage: raw Gamepad/keyboard input → the canonical pad
//! layout the control scheme binds, driven entirely by shared-engine device
//! profiles (INPUT-01).
//!
//! Every mapping decision here is DATA: which physical axis feeds which
//! canonical slot, which key drives which slot or button, and which profile a
//! connected pad resolves to (via the shared `select_by_identity`, so the
//! browser and a native host make identical selections, including the
//! fail-closed ambiguity rejection). No key list, axis index, or controller
//! table lives in shell JavaScript — the shell passes `Gamepad.id` strings,
//! raw samples, and key events through verbatim.

use pilotage_input::{
    AxisConfig, DeviceIdentity, DeviceProfile, SLOT_AXIS_BASE, axis_id_for_name,
    button_id_for_name, normalize_axis, parse_profile_bytes, select_by_identity,
};

use crate::sample::{ButtonSample, RawSample};

/// Canonical pad slot counts — the fixed geometry the scheme binds against.
pub const MAX_AXES: usize = 8;
/// Canonical button slot count.
pub const MAX_BUTTONS: usize = 24;

/// Built-in browser device profiles, embedded like the default scheme so a
/// fresh viewer maps devices with no privileged path or network fetch.
const KEYBOARD_JSON: &[u8] = include_bytes!("profiles/devices/keyboard.json");
const GENERIC_PAD_JSON: &[u8] = include_bytes!("profiles/devices/generic-pad.json");
const DUALSENSE_JSON: &[u8] = include_bytes!("profiles/devices/dualsense.json");
const RADIOMASTER_POCKET_JSON: &[u8] = include_bytes!("profiles/devices/radiomaster-pocket.json");

/// One canonical axis slot's route from a device axis, shaped by the shared
/// engine's `normalize_axis` with the profile's declared calibration.
#[derive(Debug, Clone)]
struct AxisRoute {
    source: usize,
    config: AxisConfig,
}

/// A key binding compiled to its canonical target.
#[derive(Debug, Clone)]
enum KeyTarget {
    Axis { slot: usize, value: f32 },
    Button { slot: usize },
}

#[derive(Debug, Clone)]
struct CompiledKey {
    key: String,
    target: KeyTarget,
}

/// A device profile compiled to fixed-slot routing tables, so per-tick
/// translation indexes arrays and never re-resolves names.
#[derive(Debug, Clone)]
pub(crate) struct CompiledDevice {
    label: String,
    axes: Vec<Option<AxisRoute>>,
    buttons: Vec<Option<usize>>,
    keys: Vec<CompiledKey>,
    /// Synthesized sample lengths: one past the highest mapped slot, so a
    /// keyboard tick reports the same axis/button counts as the table it
    /// replaced and goldens stay byte-identical.
    axis_len: usize,
    button_len: usize,
}

/// Why a device profile could not serve as a browser device map.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DeviceCompileError {
    /// The profile bytes failed shared-engine parsing/validation.
    Rejected,
    /// An axis or key targeted a semantic name instead of a canonical slot.
    /// A semantic-fixed device (an RC transmitter binding `roll` directly)
    /// bypasses the scheme's mode permutation, which this stage does not do.
    NotSlotTargeted,
}

impl CompiledDevice {
    fn compile(bytes: &[u8]) -> Result<Self, DeviceCompileError> {
        let profile = parse_profile_bytes(bytes).map_err(|_| DeviceCompileError::Rejected)?;
        Self::from_profile(&profile)
    }

    fn from_profile(profile: &DeviceProfile) -> Result<Self, DeviceCompileError> {
        let mut axes: Vec<Option<AxisRoute>> = vec![None; MAX_AXES];
        let mut buttons: Vec<Option<usize>> = vec![None; MAX_BUTTONS];
        let mut axis_len = 0usize;
        let mut button_len = 0usize;
        for axis in &profile.axes {
            let slot = slot_for_axis_name(&axis.logical)?;
            axes[slot] = Some(AxisRoute {
                source: axis.source_index,
                config: axis.clone(),
            });
            axis_len = axis_len.max(slot + 1);
        }
        for button in &profile.buttons {
            let slot = slot_for_button_name(&button.logical)?;
            buttons[slot] = Some(usize::from(button.source_index));
            button_len = button_len.max(slot + 1);
        }
        let mut keys = Vec::with_capacity(profile.keys.len());
        for binding in &profile.keys {
            let target = match (&binding.axis, &binding.button) {
                (Some(axis), None) => {
                    let slot = slot_for_axis_name(&axis.logical)?;
                    axis_len = axis_len.max(slot + 1);
                    KeyTarget::Axis {
                        slot,
                        value: axis.value,
                    }
                }
                (None, Some(button)) => {
                    let slot = slot_for_button_name(button)?;
                    button_len = button_len.max(slot + 1);
                    KeyTarget::Button { slot }
                }
                // parse_profile_bytes already rejected any other shape.
                _ => return Err(DeviceCompileError::Rejected),
            };
            keys.push(CompiledKey {
                key: binding.key.clone(),
                target,
            });
        }
        let label = profile
            .device
            .product
            .clone()
            .unwrap_or_else(|| profile_identity_label(profile));
        Ok(Self {
            label,
            axes,
            buttons,
            keys,
            axis_len,
            button_len,
        })
    }

    /// Translates a raw pad sample into the canonical layout: every mapped
    /// slot reads its routed, engine-normalized device axis or button; every
    /// unmapped slot reads neutral.
    fn translate_pad(&self, axes: &[f32], buttons: &[ButtonSample], out: &mut RawSample) {
        out.axes.clear();
        for route in self.axes.iter().take(MAX_AXES) {
            out.axes.push(route.as_ref().map_or(0.0, |route| {
                let raw = axes.get(route.source).copied().unwrap_or(0.0);
                normalize_axis(raw, &route.config).value
            }));
        }
        out.buttons.clear();
        for source in self.buttons.iter().take(MAX_BUTTONS) {
            out.buttons.push(
                source
                    .and_then(|source| buttons.get(source).copied())
                    .unwrap_or_default(),
            );
        }
    }

    /// Synthesizes the canonical sample from the held-key set. Bindings apply
    /// in profile order, so of two held keys driving one slot the later entry
    /// wins — the documented tie rule.
    fn synthesize_keys(&self, held: &[String], out: &mut RawSample) {
        out.axes.clear();
        out.axes.resize(self.axis_len, 0.0);
        out.buttons.clear();
        out.buttons.resize(self.button_len, ButtonSample::default());
        for binding in &self.keys {
            if !held.iter().any(|key| key == &binding.key) {
                continue;
            }
            match binding.target {
                KeyTarget::Axis { slot, value } => {
                    if let Some(axis) = out.axes.get_mut(slot) {
                        *axis = value;
                    }
                }
                KeyTarget::Button { slot } => {
                    if let Some(button) = out.buttons.get_mut(slot) {
                        *button = ButtonSample {
                            pressed: true,
                            value: 1.0,
                        };
                    }
                }
            }
        }
    }
}

fn profile_identity_label(profile: &DeviceProfile) -> String {
    format!(
        "{:04x}:{:04x}",
        profile.device.vendor_id, profile.device.product_id
    )
}

/// Resolves an axis logical name to its canonical slot, rejecting semantic
/// names (this stage routes positions; the scheme owns meaning).
fn slot_for_axis_name(name: &str) -> Result<usize, DeviceCompileError> {
    let id = axis_id_for_name(name)
        .map_err(|_| DeviceCompileError::Rejected)?
        .as_u16();
    let slot = id
        .checked_sub(SLOT_AXIS_BASE)
        .ok_or(DeviceCompileError::NotSlotTargeted)?;
    if usize::from(slot) >= MAX_AXES {
        return Err(DeviceCompileError::NotSlotTargeted);
    }
    Ok(usize::from(slot))
}

fn slot_for_button_name(name: &str) -> Result<usize, DeviceCompileError> {
    let id = button_id_for_name(name)
        .map_err(|_| DeviceCompileError::Rejected)?
        .as_u16();
    if usize::from(id) >= MAX_BUTTONS {
        return Err(DeviceCompileError::NotSlotTargeted);
    }
    Ok(usize::from(id))
}

/// How a pad selection resolved, mirrored to the shell as a plain code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectOutcome {
    /// No usable profile: the registry was ambiguous for this identity, or a
    /// built-in failed to compile. The stage keeps NO pad map — fail closed.
    Refused,
    /// An exact vendor/product profile matched.
    Exact,
    /// The generic wildcard profile was selected.
    Fallback,
}

/// The stateful device stage: the selected pad map, the keyboard map, and the
/// held-key set, all sourced from embedded profile data.
#[derive(Debug, Default)]
pub struct DeviceStage {
    pad: Option<CompiledDevice>,
    pad_outcome: Option<SelectOutcome>,
    keyboard: Option<CompiledDevice>,
    candidates: Vec<DeviceProfile>,
    held: Vec<String>,
}

impl DeviceStage {
    /// Builds the stage from the embedded profile set. A built-in that fails
    /// to parse leaves its map empty (that input source emits nothing) — the
    /// Rust test suite proves the embedded set compiles, so this is a
    /// build-integrity backstop, not an expected path.
    pub fn new() -> Self {
        let mut stage = Self {
            keyboard: CompiledDevice::compile(KEYBOARD_JSON).ok(),
            candidates: [GENERIC_PAD_JSON, DUALSENSE_JSON, RADIOMASTER_POCKET_JSON]
                .iter()
                .filter_map(|bytes| parse_profile_bytes(bytes).ok())
                .collect(),
            ..Self::default()
        };
        // A pad connected before any identity arrives maps like today's raw
        // pass-through: the wildcard standard-mapping profile.
        stage.select_pad("");
        stage
    }

    /// Resolves a `Gamepad.id` string to a device profile through the shared
    /// selector. An ambiguous registry refuses the pad entirely (no map, no
    /// control) rather than guessing.
    pub fn select_pad(&mut self, gamepad_id: &str) -> SelectOutcome {
        let identity = parse_gamepad_identity(gamepad_id);
        let outcome = match select_by_identity(identity, &self.candidates) {
            Err(_) | Ok(None) => {
                self.pad = None;
                SelectOutcome::Refused
            }
            Ok(Some(profile)) => match CompiledDevice::from_profile(profile) {
                Ok(compiled) => {
                    let exact = profile.identity() == identity;
                    self.pad = Some(compiled);
                    if exact {
                        SelectOutcome::Exact
                    } else {
                        SelectOutcome::Fallback
                    }
                }
                Err(_) => {
                    self.pad = None;
                    SelectOutcome::Refused
                }
            },
        };
        self.pad_outcome = Some(outcome);
        outcome
    }

    /// The selected pad profile's human-readable label, empty when refused.
    pub fn pad_label(&self) -> &str {
        self.pad.as_ref().map_or("", |pad| pad.label.as_str())
    }

    /// Records one key transition. Keys arrive canonical from the shell
    /// (letters lower-cased); the profile data speaks the same convention.
    pub fn key_event(&mut self, key: &str, pressed: bool) {
        if pressed {
            if !self.held.iter().any(|held| held == key) {
                self.held.push(key.to_owned());
            }
        } else {
            self.held.retain(|held| held != key);
        }
    }

    /// Drops every held key (window blur, session teardown): a key released
    /// while the page was blurred must not remain a phantom hold.
    pub fn clear_keys(&mut self) {
        self.held.clear();
    }

    /// Whether the keyboard profile binds `key` — the shell's only key
    /// question (which keys to capture from the page), answered from profile
    /// data so no key list exists outside it.
    #[must_use]
    pub fn key_is_bound(&self, key: &str) -> bool {
        self.keyboard
            .as_ref()
            .is_some_and(|keyboard| keyboard.keys.iter().any(|binding| binding.key == key))
    }

    /// Translates a raw pad sample through the selected pad map into `out`,
    /// returning the canonical axis/button counts. With no usable map
    /// (refused selection) the sample is empty and nothing can drive control.
    pub fn pad_sample(
        &self,
        axes: &[f32],
        buttons: &[ButtonSample],
        out: &mut RawSample,
    ) -> (usize, usize) {
        let Some(pad) = &self.pad else {
            out.axes.clear();
            out.buttons.clear();
            return (0, 0);
        };
        pad.translate_pad(axes, buttons, out);
        (out.axes.len(), out.buttons.len())
    }

    /// Synthesizes the canonical sample from held keys via the keyboard
    /// profile, returning the canonical axis/button counts.
    pub fn key_sample(&self, out: &mut RawSample) -> (usize, usize) {
        let Some(keyboard) = &self.keyboard else {
            out.axes.clear();
            out.buttons.clear();
            return (0, 0);
        };
        keyboard.synthesize_keys(&self.held, out);
        (out.axes.len(), out.buttons.len())
    }
}

/// Extracts the USB vendor/product identity from a browser `Gamepad.id`.
///
/// Chromium encodes `"... (Vendor: 054c Product: 0ce6)"`; Firefox encodes
/// `"054c-0ce6-DualSense Wireless Controller"`. Anything unparsable maps to
/// the wildcard identity, which resolves through the generic fallback path.
pub fn parse_gamepad_identity(id: &str) -> DeviceIdentity {
    if let Some(identity) = parse_chromium_identity(id) {
        return identity;
    }
    if let Some(identity) = parse_firefox_identity(id) {
        return identity;
    }
    DeviceIdentity::WILDCARD
}

fn parse_chromium_identity(id: &str) -> Option<DeviceIdentity> {
    let vendor = hex_after(id, "Vendor:")?;
    let product = hex_after(id, "Product:")?;
    Some(DeviceIdentity {
        vendor_id: vendor,
        product_id: product,
    })
}

fn parse_firefox_identity(id: &str) -> Option<DeviceIdentity> {
    let mut parts = id.splitn(3, '-');
    let vendor = u16::from_str_radix(parts.next()?.trim(), 16).ok()?;
    let product = u16::from_str_radix(parts.next()?.trim(), 16).ok()?;
    parts.next()?;
    Some(DeviceIdentity {
        vendor_id: vendor,
        product_id: product,
    })
}

fn hex_after(haystack: &str, marker: &str) -> Option<u16> {
    let rest = haystack.split(marker).nth(1)?.trim_start();
    let token: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    u16::from_str_radix(&token, 16).ok()
}

#[cfg(test)]
mod tests;

//! A device profile compiled to fixed-slot routing tables, so per-tick
//! translation indexes arrays and never re-resolves names (INPUT-01).

use pilotage_input::{
    AxisConfig, DeviceProfile, SLOT_AXIS_BASE, axis_id_for_name, button_id_for_name,
    content_digest, normalize_axis,
};

use crate::device::{MAX_AXES, MAX_BUTTONS};
use crate::sample::{ButtonSample, RawSample};

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
pub(crate) struct CompiledKey {
    pub(crate) key: String,
    target: KeyTarget,
}

/// A device profile compiled to fixed-slot routing tables, carrying the
/// identity metadata (label, document revision, content digest of the
/// EFFECTIVE layer-merged document) the activation announcement names.
#[derive(Debug, Clone)]
pub(crate) struct CompiledDevice {
    label: String,
    revision: u32,
    digest: [u8; 32],
    axes: Vec<Option<AxisRoute>>,
    buttons: Vec<Option<usize>>,
    /// The device's own printed name per canonical button slot, for
    /// operator-facing hints (profile `label` data; never control routing).
    button_labels: Vec<Option<String>>,
    pub(crate) keys: Vec<CompiledKey>,
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
    pub(crate) fn from_profile(profile: &DeviceProfile) -> Result<Self, DeviceCompileError> {
        let mut axes: Vec<Option<AxisRoute>> = vec![None; MAX_AXES];
        let mut buttons: Vec<Option<usize>> = vec![None; MAX_BUTTONS];
        let mut button_labels: Vec<Option<String>> = vec![None; MAX_BUTTONS];
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
            button_labels[slot] = button.label.clone();
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
        // The digest binds the EFFECTIVE document — for a layer-merged
        // profile there is no single source file, so the canonical
        // serialization of the merged result is the digested content.
        let serialized = serde_json::to_vec(profile).map_err(|_| DeviceCompileError::Rejected)?;
        Ok(Self {
            label,
            revision: profile.revision,
            digest: content_digest(&serialized),
            axes,
            buttons,
            button_labels,
            keys,
            axis_len,
            button_len,
        })
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn revision(&self) -> u32 {
        self.revision
    }

    pub(crate) fn digest(&self) -> [u8; 32] {
        self.digest
    }

    pub(crate) fn binds_key(&self, key: &str) -> bool {
        self.keys.iter().any(|binding| binding.key == key)
    }

    /// The device's printed name for a canonical button slot, if the
    /// profile declares one.
    pub(crate) fn button_label(&self, slot: usize) -> Option<&str> {
        self.button_labels.get(slot)?.as_deref()
    }

    /// The key bound to a canonical button slot. Bindings apply in profile
    /// order with later entries overriding, so the LAST binding names the
    /// key that actually drives the slot.
    pub(crate) fn key_for_button(&self, slot: usize) -> Option<&str> {
        self.keys
            .iter()
            .rev()
            .find_map(|binding| match binding.target {
                KeyTarget::Button { slot: bound } if bound == slot => Some(binding.key.as_str()),
                _ => None,
            })
    }

    /// Translates a raw pad sample into the canonical layout: every mapped
    /// slot reads its routed, engine-normalized device axis or button; every
    /// unmapped slot reads neutral.
    pub(crate) fn translate_pad(
        &self,
        axes: &[f32],
        buttons: &[ButtonSample],
        out: &mut RawSample,
    ) {
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
    pub(crate) fn synthesize_keys(&self, held: &[String], out: &mut RawSample) {
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

//! Coordinates the device stage and the control runtime as ONE activation
//! authority (INPUT-01): any effective-mapping change — new scheme bytes OR
//! a device selection that changes what physical input means — takes the
//! same transactional path: neutral output, retained same-scope authority,
//! and an advanced activation revision that the announcement and every
//! subsequent frame carry. Real scope-member transfers use the runtime's
//! separate authority-cycle path. The wasm shell and native golden harness
//! both drive this type, so their transaction semantics cannot drift.

use pilotage_input::ProfileLayer;

use crate::device::{CompiledDevice, DeviceStage, SelectOutcome};

/// Which physical source currently drives control — the identity the
/// activation announcement names. Keyboard is the boot source; a pad
/// becomes active only through a completed selection transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    /// The layered keyboard profile.
    Keyboard,
    /// The selected pad profile.
    Pad,
}

/// The parts a pending swap installs at its transaction boundary. `None`
/// leaves that part untouched.
struct PendingSwap {
    pad: Option<(Option<CompiledDevice>, SelectOutcome)>,
    keyboard: Option<Option<CompiledDevice>>,
    source: Option<InputSource>,
}
use crate::plan::ControlPlan;
use crate::profile::ProfileRuntime;
use crate::runtime::ControlRuntime;
use crate::sample::{ButtonSample, RawSample, SessionState};

/// The device-stage + runtime pair with the pending-swap bookkeeping that
/// makes a device change land only at the transaction boundary.
pub struct ControlCoordinator {
    runtime: ControlRuntime,
    stage: DeviceStage,
    /// A resolved swap (pad map, keyboard map, and/or active source)
    /// waiting for the runtime's handover to complete; installed on the
    /// tick the activation revision advances, while motion output is still
    /// gated behind the lease reacquisition.
    pending: Option<PendingSwap>,
    /// The source whose profile identity the announcement names.
    active_source: InputSource,
    /// The activation revision last observed by [`Self::evaluate`], so a
    /// revision advance (the handover completing) is detectable.
    seen_revision: u32,
    /// The last `Gamepad.id` the shell selected, so a registry change can
    /// re-resolve the SAME physical pad through the transactional path.
    last_pad_id: String,
}

impl ControlCoordinator {
    /// A coordinator with the embedded device registry and no scheme yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runtime: ControlRuntime::new(),
            stage: DeviceStage::new(),
            pending: None,
            active_source: InputSource::Keyboard,
            seen_revision: 0,
            last_pad_id: String::new(),
        }
    }

    /// Compiles and activates candidate scheme bytes through the same
    /// validated seam any source uses. Returns the activation revision on
    /// success, or `0` if the candidate failed to compile (the previously
    /// active profile, if any, stays active).
    pub fn activate_scheme(&mut self, candidate: &[u8]) -> u32 {
        match ProfileRuntime::compile(candidate) {
            Ok(compiled) => self.runtime.activate(compiled).activation_revision,
            Err(_) => 0,
        }
    }

    /// Re-opens the activation transaction for the CURRENT mapping without
    /// changing it — the seam a scope handover (e.g. entering direct
    /// flight) uses to get neutral fencing, a fresh motion generation, and a
    /// revision advance. Returns false before the first activation.
    pub fn reactivate(&mut self) -> bool {
        self.runtime.reactivate()
    }

    /// Resolves a `Gamepad.id` through the layered registry and, when the
    /// EFFECTIVE mapping changes, swaps it transactionally: the runtime
    /// re-opens its activation handover (neutral output, retained leases,
    /// revision advance) and the new map installs only when that handover
    /// completes — a deflected input on the new pad cannot drive live output
    /// until the installed map itself reads neutral. An unchanged resolution
    /// only re-seeds the discrete edge baselines.
    pub fn select_device(&mut self, gamepad_id: &str) -> SelectOutcome {
        self.last_pad_id = gamepad_id.to_owned();
        let (candidate, outcome) = self.stage.resolve_pad(gamepad_id);
        let changed = candidate.as_ref().map(CompiledDevice::digest) != self.stage.pad_digest()
            || self.active_source != InputSource::Pad;
        if !changed {
            self.stage.install_pad(candidate, outcome);
            self.runtime.reseed_edge_baselines();
            return outcome;
        }
        self.swap(PendingSwap {
            pad: Some((candidate, outcome)),
            keyboard: None,
            source: Some(InputSource::Pad),
        });
        outcome
    }

    /// The selected pad is gone (disconnect, or a physical replacement's
    /// disconnect half): control returns to the keyboard TRANSACTIONALLY —
    /// the switch changes what physical input means, so it takes the same
    /// neutral handover, retained authority, revision advance, and
    /// re-announcement a pad selection takes.
    pub fn deselect_device(&mut self) {
        self.last_pad_id = String::new();
        if self.active_source == InputSource::Keyboard && self.stage.pad_digest().is_none() {
            return;
        }
        self.swap(PendingSwap {
            pad: Some((None, SelectOutcome::Refused)),
            keyboard: None,
            source: Some(InputSource::Keyboard),
        });
    }

    /// Opens the swap transaction (or applies it immediately before the
    /// first scheme activation, when there is no authority to fence).
    fn swap(&mut self, incoming: PendingSwap) {
        if self.runtime.reactivate_mapping() {
            // A second swap opening before the first installs merges into
            // it: the transaction boundary applies the LATEST resolution of
            // every part.
            let merged = match self.pending.take() {
                Some(previous) => PendingSwap {
                    pad: incoming.pad.or(previous.pad),
                    keyboard: incoming.keyboard.or(previous.keyboard),
                    source: incoming.source.or(previous.source),
                },
                None => incoming,
            };
            self.pending = Some(merged);
        } else {
            self.install_swap(incoming);
            self.runtime.reseed_edge_baselines();
        }
    }

    fn install_swap(&mut self, swap: PendingSwap) {
        if let Some((pad, outcome)) = swap.pad {
            self.stage.install_pad(pad, outcome);
        }
        if let Some(keyboard) = swap.keyboard {
            self.stage.install_keyboard(keyboard);
        }
        if let Some(source) = swap.source {
            self.active_source = source;
        }
    }

    /// Adds a device profile to a registry layer, then re-resolves BOTH the
    /// current pad and the keyboard so an override takes the same
    /// transactional path a physical swap takes. Returns false when the
    /// bytes fail shared-engine validation.
    pub fn add_device_profile(&mut self, layer: ProfileLayer, bytes: &[u8]) -> bool {
        if !self.stage.add_profile(layer, bytes) {
            return false;
        }
        let keyboard = self.stage.resolve_keyboard();
        let keyboard_changed =
            keyboard.as_ref().map(CompiledDevice::digest) != self.stage.keyboard_digest();
        let (pad, outcome) = self.stage.resolve_pad(&self.last_pad_id.clone());
        let pad_changed = pad.as_ref().map(CompiledDevice::digest) != self.stage.pad_digest();
        if !keyboard_changed && !pad_changed {
            return true;
        }
        self.swap(PendingSwap {
            pad: pad_changed.then_some((pad, outcome)),
            keyboard: keyboard_changed.then_some(keyboard),
            source: None,
        });
        true
    }

    /// Evaluates one control tick, and lands any pending device swap the
    /// moment the runtime's handover completes (the activation revision
    /// advances): motion output remains gated until the newly installed map
    /// itself reads neutral, so an incoming-map deflection that the outgoing
    /// map could not see never publishes.
    pub fn evaluate(&mut self, sample: &RawSample, session: &SessionState) -> ControlPlan {
        let plan = self.runtime.evaluate(sample, session);
        if self.runtime.activation_revision() != self.seen_revision {
            self.seen_revision = self.runtime.activation_revision();
            if let Some(swap) = self.pending.take() {
                self.install_swap(swap);
                // The next tick re-seeds discrete baselines through the NEW
                // map, so a button held across the swap fires no edge.
                self.runtime.reseed_edge_baselines();
            }
        }
        plan
    }

    /// The current session activation revision.
    #[must_use]
    pub fn activation_revision(&self) -> u32 {
        self.runtime.activation_revision()
    }

    /// The active scheme profile's identity string (empty before activation).
    #[must_use]
    pub fn profile_id(&self) -> &str {
        self.runtime.active_profile_id()
    }

    /// The active scheme profile's DOCUMENT revision.
    #[must_use]
    pub fn profile_revision(&self) -> u32 {
        self.runtime.active_profile_revision()
    }

    /// The active scheme profile's content digest.
    #[must_use]
    pub fn profile_digest(&self) -> [u8; 32] {
        self.runtime.active_profile_digest()
    }

    /// The ACTIVE SOURCE's installed profile label: the keyboard's until a
    /// pad selection transaction completes, the pad's after — so the
    /// activation announcement always names the device actually driving.
    #[must_use]
    pub fn device_label(&self) -> &str {
        match self.active_source {
            InputSource::Keyboard => self.stage.keyboard_label(),
            InputSource::Pad => self.stage.pad_label(),
        }
    }

    /// The active source's profile document revision.
    #[must_use]
    pub fn device_revision(&self) -> u32 {
        match self.active_source {
            InputSource::Keyboard => self.stage.keyboard_revision(),
            InputSource::Pad => self.stage.pad_revision(),
        }
    }

    /// The active source's effective-content digest.
    #[must_use]
    pub fn device_digest(&self) -> Option<[u8; 32]> {
        match self.active_source {
            InputSource::Keyboard => self.stage.keyboard_digest(),
            InputSource::Pad => self.stage.pad_digest(),
        }
    }

    /// The source whose profile the announcement names.
    #[must_use]
    pub fn active_source(&self) -> InputSource {
        self.active_source
    }

    /// The operator-facing name of the ACTIVE source's arm control, from
    /// profile data: the key the keyboard binds to the scheme's arm button,
    /// or the pad's own printed button name, falling back to the canonical
    /// index. Empty before the first activation.
    #[must_use]
    pub fn arm_hint(&self) -> String {
        self.control_hint(|buttons| buttons.0)
    }

    /// The operator-facing name of the active source's disarm control.
    #[must_use]
    pub fn disarm_hint(&self) -> String {
        self.control_hint(|buttons| buttons.1)
    }

    fn control_hint(&self, pick: fn((u8, u8)) -> u8) -> String {
        let Some(buttons) = self.runtime.active_flight_buttons() else {
            return String::new();
        };
        let slot = usize::from(pick(buttons));
        match self.active_source {
            InputSource::Keyboard => self
                .stage
                .keyboard_key_for_button(slot)
                .map_or_else(|| "unbound".to_owned(), str::to_owned),
            InputSource::Pad => self
                .stage
                .pad_button_label(slot)
                .map_or_else(|| format!("button {slot}"), str::to_owned),
        }
    }

    /// Read access for sample building.
    #[must_use]
    pub fn stage(&self) -> &DeviceStage {
        &self.stage
    }

    /// Records one keyboard transition on the device stage.
    pub fn key_event(&mut self, key: &str, pressed: bool) {
        self.stage.key_event(key, pressed);
    }

    /// Drops every held key (window blur or session teardown).
    pub fn clear_keys(&mut self) {
        self.stage.clear_keys();
    }

    /// Builds a pad-sourced canonical sample through the installed pad map.
    pub fn pad_sample(
        &self,
        axes: &[f32],
        buttons: &[ButtonSample],
        out: &mut RawSample,
    ) -> (usize, usize) {
        self.stage.pad_sample(axes, buttons, out)
    }

    /// Builds a key-sourced canonical sample from held keys.
    pub fn key_sample(&self, out: &mut RawSample) -> (usize, usize) {
        self.stage.key_sample(out)
    }
}

impl Default for ControlCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

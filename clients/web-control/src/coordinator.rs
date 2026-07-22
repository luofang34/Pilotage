//! Coordinates the device stage and the control runtime as ONE activation
//! authority (INPUT-01): any effective-mapping change — new scheme bytes OR
//! a device selection that changes what physical input means — takes the
//! same transactional path: neutral output, gimbal + motion lease cycle,
//! and an advanced activation revision that the announcement and every
//! subsequent frame carry. The wasm shell and the native golden harness
//! both drive this type, so their transaction semantics cannot drift.

use pilotage_input::ProfileLayer;

use crate::device::{CompiledDevice, DeviceStage, SelectOutcome};
use crate::plan::ControlPlan;
use crate::profile::ProfileRuntime;
use crate::runtime::ControlRuntime;
use crate::sample::{ButtonSample, RawSample, SessionState};

/// The device-stage + runtime pair with the pending-swap bookkeeping that
/// makes a device change land only at the transaction boundary.
pub struct ControlCoordinator {
    runtime: ControlRuntime,
    stage: DeviceStage,
    /// A resolved device selection waiting for the runtime's handover to
    /// complete; installed on the tick the activation revision advances,
    /// while motion output is still gated behind the lease reacquisition.
    pending_pad: Option<(Option<CompiledDevice>, SelectOutcome)>,
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
            pending_pad: None,
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

    /// Resolves a `Gamepad.id` through the layered registry and, when the
    /// EFFECTIVE mapping changes, swaps it transactionally: the runtime
    /// re-opens its activation handover (neutral output, lease cycle,
    /// revision advance) and the new map installs only when that handover
    /// completes — a deflected input on the new pad cannot drive the
    /// existing lease for even one tick. An unchanged resolution only
    /// re-seeds the discrete edge baselines.
    pub fn select_device(&mut self, gamepad_id: &str) -> SelectOutcome {
        self.last_pad_id = gamepad_id.to_owned();
        let (candidate, outcome) = self.stage.resolve_pad(gamepad_id);
        let changed = candidate.as_ref().map(CompiledDevice::digest) != self.stage.pad_digest();
        if !changed {
            self.stage.install_pad(candidate, outcome);
            self.runtime.reseed_edge_baselines();
            return outcome;
        }
        if self.runtime.reactivate() {
            self.pending_pad = Some((candidate, outcome));
        } else {
            // No scheme active yet: there is no authority to fence and no
            // revision to advance until one installs.
            self.stage.install_pad(candidate, outcome);
            self.runtime.reseed_edge_baselines();
        }
        outcome
    }

    /// Adds a device profile to a registry layer, then re-resolves the
    /// current pad so an override takes the same transactional path a
    /// physical swap takes. Returns false when the bytes fail shared-engine
    /// validation.
    pub fn add_device_profile(&mut self, layer: ProfileLayer, bytes: &[u8]) -> bool {
        if !self.stage.add_profile(layer, bytes) {
            return false;
        }
        let last = self.last_pad_id.clone();
        self.select_device(&last);
        true
    }

    /// Evaluates one control tick, and lands any pending device swap the
    /// moment the runtime's handover completes (the activation revision
    /// advances): motion output is still gated behind the motion-lease
    /// reacquisition at that point, so the remapped pad never publishes on
    /// the old authority.
    pub fn evaluate(&mut self, sample: &RawSample, session: &SessionState) -> ControlPlan {
        let plan = self.runtime.evaluate(sample, session);
        if self.runtime.activation_revision() != self.seen_revision {
            self.seen_revision = self.runtime.activation_revision();
            if let Some((pad, outcome)) = self.pending_pad.take() {
                self.stage.install_pad(pad, outcome);
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

    /// The INSTALLED pad profile's label (empty while refused or pending).
    #[must_use]
    pub fn device_label(&self) -> &str {
        self.stage.pad_label()
    }

    /// The installed pad profile's document revision (zero when none).
    #[must_use]
    pub fn device_revision(&self) -> u32 {
        self.stage.pad_revision()
    }

    /// The installed pad profile's effective-content digest (None when no
    /// pad map is installed).
    #[must_use]
    pub fn device_digest(&self) -> Option<[u8; 32]> {
        self.stage.pad_digest()
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

//! Atomic production of all panel scenes in the screen composition.

use indicate_alerts::{AlertContext, ManagerHealth};
use indicate_instrument_registry::{ConfigBlob, PanelDrawError};
use indicate_instrument_scene::SceneWriter;
use indicate_instrument_state::abi::v7::{self, AbiError};
use indicate_instrument_state::{AircraftState, FreshnessPolicy, PanelData, Stamped};

use crate::RenderStatus;
use crate::composition::composition;
use crate::registry::{canonical_frame, descriptor, panel_index};
use crate::runtime::{
    AlertStepOutcome, Runtime, derive_alert_events, scene_error_status, validate_panel_scene,
};

/// The result for one panel in a composition transaction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompositionPanelOutcome {
    /// The panel index in the runtime registry.
    pub panel: u32,
    /// The status of the complete composition transaction.
    pub status: RenderStatus,
    /// The panel scene offset in the composition scene buffer.
    pub scene_offset: u32,
    /// The panel scene length in bytes.
    pub scene_len: u32,
    /// The width used to produce the panel scene.
    pub frame_width: f32,
    /// The height used to produce the panel scene.
    pub frame_height: f32,
    /// The panel generation after a successful transaction.
    pub generation: u32,
}

impl CompositionPanelOutcome {
    pub(crate) const fn empty() -> Self {
        Self {
            panel: 0,
            status: RenderStatus::InvalidPanel,
            scene_offset: 0,
            scene_len: 0,
            frame_width: 0.0,
            frame_height: 0.0,
            generation: 0,
        }
    }
}

/// The result of one complete screen-composition transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompositionFrameOutcome {
    /// The status of the transaction.
    pub status: RenderStatus,
    /// The committed composition scene length. This is zero on failure.
    pub scene_len: u32,
    /// The composition generation. It advances only on full success.
    pub generation: u32,
    /// The alert result from the same state and clock.
    pub alerts: AlertStepOutcome,
}

impl CompositionFrameOutcome {
    fn failure(status: RenderStatus, generation: u32) -> Self {
        Self {
            status,
            scene_len: 0,
            generation,
            alerts: AlertStepOutcome::failure(status),
        }
    }
}

impl Runtime {
    /// Returns the committed bytes from the last successful composition.
    pub fn composition_scene(&self) -> &[u8] {
        &self.composition_scene
    }

    /// Returns the panel results from the last composition attempt.
    pub fn composition_panel_outcomes(&self) -> &[CompositionPanelOutcome] {
        &self.composition_panels
    }

    /// Produces every panel from one aged state and one alert step.
    ///
    /// `age_delta_ms` is the elapsed monotonic time since the state frame
    /// was accepted. `now_ms` is the same clock at the frame boundary.
    pub fn render_composition(
        &mut self,
        age_delta_ms: u64,
        now_ms: u64,
        path_healthy: bool,
    ) -> CompositionFrameOutcome {
        if !self.reset_composition_panels() {
            return self.fail_composition(RenderStatus::InvalidPanel);
        }
        let report = match v7::decode_state(&self.state) {
            Ok(report) => report,
            Err(error) => return self.fail_composition(abi_error_status(error)),
        };
        let mut state = report.state;
        age_state(&mut state, age_delta_ms);

        let mut unusual = self.unusual;
        let data = indicate_instrument_state::resolve_stateful(
            &state,
            &FreshnessPolicy::default(),
            &self.profile,
            &mut unusual,
        );
        let (alerts, alert_output, alert_outcome) = self.stage_alerts(&data, now_ms, path_healthy);
        if let Err(status) = self.draw_composition(&data, &alert_output) {
            return self.fail_composition(status);
        }

        self.unusual = unusual;
        self.alerts = alerts;
        self.alert_output = Some(alert_output);
        self.unknown_groups = self
            .unknown_groups
            .wrapping_add(u32::from(report.unknown_groups));
        self.extended_groups = self
            .extended_groups
            .wrapping_add(u32::from(report.extended_groups));
        let Some(scene_len) = self.commit_composition() else {
            return self.fail_composition(RenderStatus::InvalidPanel);
        };
        CompositionFrameOutcome {
            status: RenderStatus::Ok,
            scene_len,
            generation: self.composition_generation,
            alerts: alert_outcome,
        }
    }

    fn stage_alerts(
        &self,
        data: &PanelData,
        now_ms: u64,
        path_healthy: bool,
    ) -> (
        indicate_alerts::AlertManager,
        indicate_alerts::AlertOutput,
        AlertStepOutcome,
    ) {
        let mut manager = self.alerts.clone();
        let context = AlertContext {
            declutter: data.presentation.unusual,
            alerting_path_healthy: path_healthy,
            ..AlertContext::default()
        };
        let output = manager.step(
            &self.alert_profile,
            &derive_alert_events(data),
            context,
            now_ms,
        );
        let outcome = AlertStepOutcome {
            status: RenderStatus::Ok,
            active_count: output.active().len() as u32,
            faulted: output.health() == ManagerHealth::Faulted,
            overflow: output.overflow(),
            manager_generation: output.generation(),
        };
        (manager, output, outcome)
    }

    fn draw_composition(
        &mut self,
        data: &PanelData,
        alerts: &indicate_alerts::AlertOutput,
    ) -> Result<(), RenderStatus> {
        let Some(composition) = composition() else {
            return Err(RenderStatus::InvalidPanel);
        };
        if self.composition_panels.len() != composition.slots.len() {
            return Err(RenderStatus::InvalidPanel);
        }
        let mut offset = 0usize;
        for (slot_idx, slot) in composition.slots.iter().enumerate() {
            let Some(panel_idx) = panel_index(slot.panel) else {
                return Err(RenderStatus::InvalidPanel);
            };
            let len = self.draw_panel(panel_idx, data, alerts)?;
            let Some(end) = offset.checked_add(len) else {
                return Err(RenderStatus::SceneBufferFull);
            };
            let Some(target) = self.composition_scene.get_mut(offset..end) else {
                return Err(RenderStatus::SceneBufferFull);
            };
            target.copy_from_slice(&self.scene[..len]);
            self.composition_panels[slot_idx] = self.panel_outcome(panel_idx, offset, len);
            offset = end;
        }
        Ok(())
    }

    fn reset_composition_panels(&mut self) -> bool {
        let Some(composition) = composition() else {
            return false;
        };
        if self.composition_panels.len() != composition.slots.len() {
            return false;
        }
        for (outcome, slot) in self
            .composition_panels
            .iter_mut()
            .zip(composition.slots.iter())
        {
            let Some(panel_idx) = panel_index(slot.panel) else {
                return false;
            };
            let frame = descriptor(panel_idx as u32).map(canonical_frame);
            *outcome = CompositionPanelOutcome {
                panel: panel_idx as u32,
                status: RenderStatus::InvalidPanel,
                scene_offset: 0,
                scene_len: 0,
                frame_width: frame.map_or(0.0, |value| value.width),
                frame_height: frame.map_or(0.0, |value| value.height),
                generation: self.generation.get(panel_idx).copied().unwrap_or(0),
            };
        }
        true
    }

    fn draw_panel(
        &mut self,
        panel_idx: usize,
        data: &PanelData,
        alerts: &indicate_alerts::AlertOutput,
    ) -> Result<usize, RenderStatus> {
        let Some(panel) = descriptor(panel_idx as u32) else {
            return Err(RenderStatus::InvalidPanel);
        };
        let config_bytes = self.config.get(panel_idx).map_or(&[][..], Vec::as_slice);
        let config = ConfigBlob::parse(config_bytes).map_err(|_| RenderStatus::ConfigInvalid)?;
        let mut writer = SceneWriter::new(&mut self.scene).map_err(scene_error_status)?;
        let frame = canonical_frame(panel);
        let len = match (panel.draw)(data, &config, Some(alerts), frame, &mut writer) {
            Ok(()) => writer.finish(),
            Err(PanelDrawError::Scene(error)) => return Err(scene_error_status(error)),
            Err(PanelDrawError::Config(_)) => return Err(RenderStatus::ConfigInvalid),
        };
        let status = validate_panel_scene(panel_idx, &self.scene[..len]);
        if status != RenderStatus::Ok {
            return Err(status);
        }
        Ok(len)
    }

    fn panel_outcome(
        &self,
        panel_idx: usize,
        offset: usize,
        len: usize,
    ) -> CompositionPanelOutcome {
        let frame = descriptor(panel_idx as u32).map(canonical_frame);
        CompositionPanelOutcome {
            panel: panel_idx as u32,
            status: RenderStatus::Ok,
            scene_offset: offset as u32,
            scene_len: len as u32,
            frame_width: frame.map_or(0.0, |value| value.width),
            frame_height: frame.map_or(0.0, |value| value.height),
            generation: self.panel_generation(panel_idx),
        }
    }

    fn commit_composition(&mut self) -> Option<u32> {
        if self
            .composition_panels
            .iter()
            .any(|panel| self.generation.get(panel.panel as usize).is_none())
        {
            return None;
        }
        let mut scene_len = 0u32;
        for panel in &mut self.composition_panels {
            let panel_idx = panel.panel as usize;
            if let Some(generation) = self.generation.get_mut(panel_idx) {
                *generation = generation.wrapping_add(1);
                panel.generation = *generation;
            }
            scene_len = scene_len.saturating_add(panel.scene_len);
        }
        self.composition_generation = self.composition_generation.wrapping_add(1);
        Some(scene_len)
    }

    fn fail_composition(&mut self, status: RenderStatus) -> CompositionFrameOutcome {
        for panel in &mut self.composition_panels {
            panel.status = status;
            panel.scene_offset = 0;
            panel.scene_len = 0;
            panel.generation = self
                .generation
                .get(panel.panel as usize)
                .copied()
                .unwrap_or(0);
        }
        CompositionFrameOutcome::failure(status, self.composition_generation)
    }
}

fn abi_error_status(error: AbiError) -> RenderStatus {
    match error {
        AbiError::Truncated => RenderStatus::StateTruncated,
        AbiError::BadVersion { .. } => RenderStatus::StateBadVersion,
        AbiError::NonCanonicalOrder { .. } | AbiError::GroupTruncated { .. } => {
            RenderStatus::StateMalformed
        }
    }
}

fn age_state(state: &mut AircraftState, delta_ms: u64) {
    let delta = delta_ms as f32;
    age_stamp(&mut state.attitude, delta);
    age_stamp(&mut state.kinematics, delta);
    age_stamp(&mut state.air, delta);
    age_stamp(&mut state.nav, delta);
    age_stamp(&mut state.wind, delta);
    age_stamp(&mut state.heading, delta);
    age_stamp(&mut state.variation, delta);
    age_stamp(&mut state.dynamics, delta);
    age_stamp(&mut state.director, delta);
    age_stamp(&mut state.monitor_text, delta);
}

fn age_stamp<T>(stamp: &mut Stamped<T>, delta_ms: f32) {
    if let Some(age_ms) = stamp.age_ms
        && age_ms.is_finite()
        && age_ms >= 0.0
    {
        stamp.age_ms = Some((age_ms + delta_ms).min(f32::MAX));
    }
}

#[cfg(test)]
mod tests;

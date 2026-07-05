//! Wraps `pilotage-input`'s normalization pipeline: turns a raw device
//! sample into a [`ControlPayload`] using a loaded [`DeviceProfile`].

use pilotage_input::{
    ButtonTracker, DeviceProfile, RawDeviceSample, axis_id_for_name, button_id_for_name,
    normalize_axis,
};
use pilotage_protocol::{ButtonEdge as ProtoButtonEdge, ControlPayload};

use crate::error::ProbeError;

/// Loads the RadioMaster Pocket device profile embedded at compile time
/// from `crates/pilotage-input/registry/radiomaster-pocket.json`.
///
/// # Errors
///
/// Returns [`ProbeError::Profile`] if the embedded JSON fails to parse or
/// validate (only possible if the registry file itself is corrupted).
pub fn load_radiomaster_pocket_profile() -> Result<DeviceProfile, ProbeError> {
    const PROFILE_JSON: &str =
        include_str!("../../../crates/pilotage-input/registry/radiomaster-pocket.json");
    Ok(pilotage_input::load_profile_str(PROFILE_JSON)?)
}

/// Stateful pipeline stage: holds the button-edge tracker across samples so
/// held buttons emit exactly one press and one release edge, per
/// `pilotage_input::ButtonTracker`'s contract.
pub struct Pipeline {
    profile: DeviceProfile,
    tracker: ButtonTracker,
}

impl Pipeline {
    /// Constructs a pipeline bound to `profile`, with no buttons held.
    #[must_use]
    pub const fn new(profile: DeviceProfile) -> Self {
        Self {
            profile,
            tracker: ButtonTracker::new(),
        }
    }

    /// Runs `sample` through calibration/deadzone/expo/invert normalization
    /// for every configured axis and edge-detects every configured button,
    /// producing the [`ControlPayload`] a control frame carries.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeError::Profile`] if the profile references a logical
    /// axis or button name outside the well-known table (`pilotage_input`
    /// already validates this at load time, so this only re-surfaces a
    /// defect in an embedded profile, not a runtime device condition).
    pub fn normalize(&mut self, sample: &RawDeviceSample) -> Result<ControlPayload, ProbeError> {
        let axes = self.normalize_axes(sample)?;
        let edges = self.normalize_buttons(sample)?;
        Ok(ControlPayload { axes, edges })
    }

    fn normalize_axes(
        &self,
        sample: &RawDeviceSample,
    ) -> Result<Vec<(pilotage_protocol::LogicalAxisId, f32)>, ProbeError> {
        self.profile
            .axes
            .iter()
            .map(|axis| {
                let raw = sample.axes.get(axis.source_index).copied().unwrap_or(0.0);
                let normalized = normalize_axis(raw, axis);
                let id = axis_id_for_name(&axis.logical)?;
                Ok((id, normalized.value))
            })
            .collect()
    }

    fn normalize_buttons(
        &mut self,
        sample: &RawDeviceSample,
    ) -> Result<Vec<(pilotage_protocol::LogicalButtonId, ProtoButtonEdge)>, ProbeError> {
        let edges = self.tracker.update(sample.buttons);
        edges
            .into_iter()
            .map(|(source_index, edge)| {
                let logical = self
                    .profile
                    .buttons
                    .iter()
                    .find(|button| button.source_index == source_index)
                    .map(|button| button.logical.as_str());
                match logical {
                    Some(name) => Ok((button_id_for_name(name)?, edge)),
                    None => Ok((
                        pilotage_protocol::LogicalButtonId::new(u16::from(source_index)),
                        edge,
                    )),
                }
            })
            .collect()
    }

    /// Returns the profile revision, carried on outgoing control frames so
    /// the host knows which calibration produced a given frame.
    #[must_use]
    pub const fn profile_revision(&self) -> u32 {
        self.profile.revision
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Pipeline, load_radiomaster_pocket_profile};
    use pilotage_input::RawDeviceSample;
    use pilotage_timing::MonoTimestamp;

    #[test]
    fn builtin_profile_loads() {
        let profile = load_radiomaster_pocket_profile().expect("profile loads");
        assert_eq!(profile.axes.len(), 8);
        assert_eq!(profile.buttons.len(), 24);
    }

    #[test]
    fn normalize_produces_centered_axes_at_rest() {
        let profile = load_radiomaster_pocket_profile().expect("profile loads");
        let mut pipeline = Pipeline::new(profile);
        let sample = RawDeviceSample::new(
            vec![1024.0, 1024.0, 0.0, 1024.0, 0.0, 0.0, 0.0, 0.0],
            0,
            MonoTimestamp::from_nanos(0),
        );
        let payload = pipeline.normalize(&sample).expect("normalizes");
        assert_eq!(payload.axes.len(), 8);
        for (_, value) in &payload.axes {
            assert!(
                value.abs() < 1e-3,
                "expected near-zero at rest, got {value}"
            );
        }
        assert!(payload.edges.is_empty());
    }

    #[test]
    fn normalize_emits_button_edge_on_press() {
        let profile = load_radiomaster_pocket_profile().expect("profile loads");
        let mut pipeline = Pipeline::new(profile);
        let idle = RawDeviceSample::new(vec![0.0; 8], 0, MonoTimestamp::from_nanos(0));
        let pressed = RawDeviceSample::new(vec![0.0; 8], 1, MonoTimestamp::from_nanos(1));
        pipeline.normalize(&idle).expect("normalizes");
        let payload = pipeline.normalize(&pressed).expect("normalizes");
        assert_eq!(payload.edges.len(), 1);
    }
}

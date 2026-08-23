//! Atomic control-feel profile selection.

use pilotage_control_feel::{FeelDigest, FeelMode, ValidatedFlightFeelProfile};
use pilotage_protocol::{ControlIntent, ScopedControlFrame};

use crate::error::AviateAdapterError;

/// Identity of one validated control-feel artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ControlFeelIdentity {
    pub(super) profile_id: String,
    pub(super) mode: FeelMode,
    pub(super) schema: u16,
    pub(super) digest: FeelDigest,
}

#[derive(Debug, Clone)]
pub(super) struct ControlFeelEntry {
    pub(super) profile: ValidatedFlightFeelProfile,
    pub(super) identity: ControlFeelIdentity,
}

impl ControlFeelEntry {
    fn new(profile: ValidatedFlightFeelProfile) -> Result<Self, AviateAdapterError> {
        let identity = ControlFeelIdentity {
            profile_id: profile.profile().profile_id.clone(),
            mode: profile.profile().mode,
            schema: profile.profile().schema_version,
            digest: FeelDigest::calculate(&profile)
                .map_err(|source| AviateAdapterError::ControlFeelIdentity { source })?,
        };
        Ok(Self { profile, identity })
    }
}

/// Active, pending, and rollback control-feel artifacts.
#[derive(Debug)]
pub(super) struct ControlFeelProfiles {
    active: ControlFeelEntry,
    pending: Option<ControlFeelEntry>,
    previous: Option<ControlFeelEntry>,
}

impl ControlFeelProfiles {
    pub(super) fn new(profile: ValidatedFlightFeelProfile) -> Result<Self, AviateAdapterError> {
        Ok(Self {
            active: ControlFeelEntry::new(profile)?,
            pending: None,
            previous: None,
        })
    }

    pub(super) fn active(&self) -> &ControlFeelEntry {
        &self.active
    }

    pub(super) fn stage(
        &mut self,
        profile: ValidatedFlightFeelProfile,
    ) -> Result<FeelDigest, AviateAdapterError> {
        let entry = ControlFeelEntry::new(profile)?;
        let digest = entry.identity.digest;
        self.pending = Some(entry);
        Ok(digest)
    }

    pub(super) fn stage_rollback(&mut self) -> bool {
        let Some(previous) = self.previous.clone() else {
            return false;
        };
        self.pending = Some(previous);
        true
    }

    pub(super) fn pending_is_neutral(&self, frame: &ScopedControlFrame) -> bool {
        let Some(pending) = self.pending.as_ref() else {
            return false;
        };
        frame_is_neutral(frame, &self.active.profile) && frame_is_neutral(frame, &pending.profile)
    }

    pub(super) fn commit_pending(&mut self) -> Option<ControlFeelEntry> {
        let next = self.pending.take()?;
        self.previous = Some(core::mem::replace(&mut self.active, next.clone()));
        Some(next)
    }
}

fn frame_is_neutral(frame: &ScopedControlFrame, profile: &ValidatedFlightFeelProfile) -> bool {
    if !frame.actions.is_empty() {
        return false;
    }
    let profile = profile.profile();
    match &frame.intent {
        Some(ControlIntent::Velocity(value)) => {
            curved_is_neutral(
                value.vx,
                profile.envelope.horizontal_speed_mps,
                profile.horizontal,
            ) && curved_is_neutral(
                value.vy,
                profile.envelope.horizontal_speed_mps,
                profile.horizontal,
            ) && curved_is_neutral(
                value.vz,
                profile.envelope.vertical_speed_mps,
                profile.vertical,
            ) && curved_is_neutral(value.yaw_rate, profile.envelope.yaw_rate_rps, profile.yaw)
        }
        Some(ControlIntent::AttitudeThrust(value)) => {
            let (roll, pitch, _) = pilotage_adapter_api::attitude_euler(value);
            let tilt_neutral =
                profile.envelope.direct_tilt_rad * profile.horizontal.neutral.active_exit;
            let thrust_neutral = 0.5 * profile.vertical.neutral.active_exit;
            roll.abs() <= tilt_neutral
                && pitch.abs() <= tilt_neutral
                && (value.thrust - 0.5).abs() <= thrust_neutral
        }
        _ => false,
    }
}

fn curved_is_neutral(
    demand: f32,
    scale: f32,
    response: pilotage_control_feel::AxisResponse,
) -> bool {
    demand.is_finite() && response.curve.apply(demand / scale).abs() <= response.neutral.active_exit
}

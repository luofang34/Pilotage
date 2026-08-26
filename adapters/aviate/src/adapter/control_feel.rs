//! Atomic control-feel profile selection.

use std::time::Instant;

use pilotage_control_feel::{
    AxisResponse, FeelDigest, FeelMode, NeutralBand, NeutralLatch, ValidatedFlightFeelProfile,
};
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

#[derive(Debug, Clone)]
struct StagedControlFeel {
    entry: ControlFeelEntry,
    neutral: NeutralBoundary,
}

#[derive(Debug, Clone, Default)]
struct NeutralBoundary {
    horizontal: [NeutralObservation; 2],
    vertical: NeutralObservation,
    yaw: NeutralObservation,
    direct_tilt: [NeutralObservation; 2],
    direct_thrust: NeutralObservation,
    family: Option<NeutralFamily>,
}

#[derive(Debug, Clone, Copy, Default)]
struct NeutralObservation {
    latch: NeutralLatch,
    last_sample_at: Option<Instant>,
    exit_eligible: bool,
    initialized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NeutralFamily {
    Velocity,
    Direct,
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
    active_neutral: NeutralBoundary,
    pending: Option<StagedControlFeel>,
    previous: Option<ControlFeelEntry>,
}

impl ControlFeelProfiles {
    pub(super) fn new(profile: ValidatedFlightFeelProfile) -> Result<Self, AviateAdapterError> {
        Ok(Self {
            active: ControlFeelEntry::new(profile)?,
            active_neutral: NeutralBoundary::default(),
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
        self.pending = Some(StagedControlFeel {
            entry,
            neutral: NeutralBoundary::default(),
        });
        Ok(digest)
    }

    pub(super) fn stage_rollback(&mut self) -> bool {
        let Some(previous) = self.previous.clone() else {
            return false;
        };
        self.pending = Some(StagedControlFeel {
            entry: previous,
            neutral: NeutralBoundary::default(),
        });
        true
    }

    pub(super) fn pending_is_neutral(
        &mut self,
        frame: &ScopedControlFrame,
        observed_at: Instant,
    ) -> bool {
        let active_is_neutral =
            self.active_neutral
                .update(frame, &self.active.profile, observed_at);
        let Some(pending) = self.pending.as_mut() else {
            return false;
        };
        let pending_is_neutral = pending
            .neutral
            .update(frame, &pending.entry.profile, observed_at);
        active_is_neutral && pending_is_neutral
    }

    /// Drops a staged law that was never installed.
    ///
    /// A law staged under a lease that has since been lost is a law the next
    /// operator did not choose, and it would install itself on their first
    /// sustained neutral. The same reasoning already discards a hold point
    /// captured under a lost lease: what was asked for under one authority
    /// must not act under another.
    ///
    /// Returns whether anything was pending.
    pub(super) fn discard_pending(&mut self) -> bool {
        self.pending.take().is_some()
    }

    pub(super) fn commit_pending(&mut self) -> Option<ControlFeelEntry> {
        let staged = self.pending.take()?;
        let next = staged.entry;
        self.previous = Some(core::mem::replace(&mut self.active, next.clone()));
        self.active_neutral = staged.neutral;
        Some(next)
    }
}

impl NeutralBoundary {
    fn update(
        &mut self,
        frame: &ScopedControlFrame,
        profile: &ValidatedFlightFeelProfile,
        observed_at: Instant,
    ) -> bool {
        let profile = profile.profile();
        let neutral = match &frame.intent {
            Some(ControlIntent::Velocity(value)) => {
                self.select_family(NeutralFamily::Velocity);
                let horizontal = [value.vx, value.vy];
                let mut neutral = true;
                for (observation, demand) in self.horizontal.iter_mut().zip(horizontal) {
                    neutral &= observation.update(
                        curved_magnitude(
                            demand,
                            profile.envelope.horizontal_speed_mps,
                            profile.horizontal,
                        ),
                        observed_at,
                        profile.horizontal.neutral,
                    );
                }
                neutral &= self.vertical.update(
                    curved_magnitude(
                        value.vz,
                        profile.envelope.vertical_speed_mps,
                        profile.vertical,
                    ),
                    observed_at,
                    profile.vertical.neutral,
                );
                neutral &= self.yaw.update(
                    curved_magnitude(value.yaw_rate, profile.envelope.yaw_rate_rps, profile.yaw),
                    observed_at,
                    profile.yaw.neutral,
                );
                neutral
            }
            Some(ControlIntent::AttitudeThrust(value)) => {
                self.select_family(NeutralFamily::Direct);
                let (roll, pitch, _) = pilotage_adapter_api::attitude_euler(value);
                let mut neutral = true;
                for (observation, demand) in self.direct_tilt.iter_mut().zip([roll, pitch]) {
                    neutral &= observation.update(
                        normalized_magnitude(demand, profile.envelope.direct_tilt_rad),
                        observed_at,
                        profile.horizontal.neutral,
                    );
                }
                neutral &= self.direct_thrust.update(
                    normalized_magnitude(value.thrust - 0.5, 0.5),
                    observed_at,
                    profile.vertical.neutral,
                );
                neutral
            }
            _ => {
                self.family = None;
                false
            }
        };
        frame.actions.is_empty() && neutral
    }

    fn select_family(&mut self, family: NeutralFamily) {
        if self.family == Some(family) {
            return;
        }
        match family {
            NeutralFamily::Velocity => {
                self.horizontal = Default::default();
                self.vertical = Default::default();
                self.yaw = Default::default();
            }
            NeutralFamily::Direct => {
                self.direct_tilt = Default::default();
                self.direct_thrust = Default::default();
            }
        }
        self.family = Some(family);
    }
}

impl NeutralObservation {
    fn update(&mut self, magnitude: f32, observed_at: Instant, band: NeutralBand) -> bool {
        if !self.initialized {
            self.latch.update(1.0, 0.0, band);
            self.initialized = true;
        }
        let magnitude = if magnitude.is_finite() {
            magnitude.abs()
        } else {
            1.0
        };
        let exit_eligible = magnitude <= band.active_exit;
        let dt_s = if self.exit_eligible && exit_eligible {
            self.last_sample_at
                .and_then(|last| observed_at.checked_duration_since(last))
                .map_or(0.0, |elapsed| elapsed.as_secs_f32())
                .clamp(0.0, crate::uplink::MAX_DT_S)
        } else {
            0.0
        };
        self.last_sample_at = Some(observed_at);
        self.exit_eligible = exit_eligible;
        !self.latch.update(magnitude, dt_s, band)
    }
}

fn curved_magnitude(demand: f32, scale: f32, response: AxisResponse) -> f32 {
    if !demand.is_finite() || !scale.is_finite() || scale <= 0.0 {
        return f32::INFINITY;
    }
    response.curve.apply(demand / scale).abs()
}

fn normalized_magnitude(demand: f32, scale: f32) -> f32 {
    if !demand.is_finite() || !scale.is_finite() || scale <= 0.0 {
        f32::INFINITY
    } else {
        (demand / scale).abs()
    }
}

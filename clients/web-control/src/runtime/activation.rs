//! Profile activation and neutral handover transactions.

use super::authority::{MOTION_LEASE_RETRY_MS, MotionPhase};
use super::{ControlRuntime, NEVER_REQUESTED_MS, controls_neutral, neutral_gimbal, neutral_motion};
use crate::plan::{ActivationPlan, ControlPlan, LeaseAction};
use crate::profile::CompiledProfile;
use crate::quasimode::reset_held;
use crate::sample::{RawSample, SessionState};

impl ControlRuntime {
    /// Begins a same-scope profile activation. Existing authority stays held.
    pub fn activate(&mut self, candidate: CompiledProfile) -> ActivationPlan {
        self.begin_activation(candidate, false)
    }

    /// Re-opens activation for a device map while retaining both leases.
    pub fn reactivate_mapping(&mut self) -> bool {
        self.reactivate_active(false)
    }

    /// Re-opens activation for a motion scope-member transfer.
    pub fn reactivate(&mut self) -> bool {
        self.reactivate_active(true)
    }

    fn reactivate_active(&mut self, releases_motion: bool) -> bool {
        match self.active.clone() {
            Some(active) => {
                self.begin_activation(active, releases_motion);
                true
            }
            None => false,
        }
    }

    fn begin_activation(
        &mut self,
        candidate: CompiledProfile,
        releases_motion: bool,
    ) -> ActivationPlan {
        self.streaming = false;
        if self.active.is_none() {
            self.install(candidate);
            return ActivationPlan {
                installed: true,
                activation_revision: self.activation_revision,
                emit_neutral: true,
                release_gimbal_lease: false,
                release_motion_lease: false,
            };
        }
        let transaction_open = self.pending.is_some();
        self.pending = Some(candidate);
        self.handover_releases_motion = if transaction_open {
            self.handover_releases_motion || releases_motion
        } else {
            releases_motion
        };
        if releases_motion {
            // A scope transfer has emitted no motion release yet.
            self.motion_action_ms = NEVER_REQUESTED_MS;
        }
        ActivationPlan {
            installed: false,
            activation_revision: self.activation_revision,
            emit_neutral: true,
            release_gimbal_lease: false,
            release_motion_lease: releases_motion,
        }
    }

    /// Installs the first profile and initializes its edge baselines.
    fn install(&mut self, candidate: CompiledProfile) {
        self.active = Some(candidate);
        self.pending = None;
        self.activation_revision = self.activation_revision.wrapping_add(1);
        self.reset_baseline = false;
        self.streaming = false;
        self.last_request_ms = NEVER_REQUESTED_MS;
        self.prev_arm = false;
        self.prev_disarm = false;
        self.mapping_neutral_pending = false;
        self.handover_releases_motion = false;
    }

    /// Emits neutral until every control used by either profile is centered:
    /// a swap remaps flight and gimbal inputs, so any input deflected under
    /// EITHER profile could jump meaning at install. On the transfer path the
    /// motion release is emitted only while the session still shows the lease
    /// granted, and re-emitted on [`MOTION_LEASE_RETRY_MS`] — a lost write
    /// must not wedge the handover, while a release per tick would draw a
    /// `released: false` acknowledgement (a host warning) for as long as the
    /// operator stays deflected.
    pub(super) fn evaluate_handover(
        &mut self,
        sample: &RawSample,
        session: &SessionState,
        active: &CompiledProfile,
    ) -> ControlPlan {
        let releases_motion = self.handover_releases_motion;
        let release_motion = releases_motion
            && session.motion_granted
            && session.now_ms - self.motion_action_ms >= MOTION_LEASE_RETRY_MS;
        if release_motion {
            self.motion_action_ms = session.now_ms;
        }
        let union_neutral = controls_neutral(sample, active)
            && self
                .pending
                .as_ref()
                .is_some_and(|candidate| controls_neutral(sample, candidate));
        let installed = match union_neutral.then(|| self.pending.take()).flatten() {
            Some(candidate) => {
                self.reset_baseline = reset_held(sample, &candidate.gimbal);
                self.prev_arm = sample.pressed(usize::from(candidate.flight.arm_button));
                self.prev_disarm = sample.pressed(usize::from(candidate.flight.disarm_button));
                self.install_after_seed(candidate, session.now_ms, releases_motion);
                true
            }
            None => false,
        };
        ControlPlan {
            // The install tick emits NO frames: the activation revision just
            // advanced, frames are datagrams, and a datagram under the new
            // revision could beat the shell's reliable-stream re-announcement
            // to the host (a ProfileMismatch rejection). One silent tick gives
            // the announcement a head start the liveness watchdog comfortably
            // tolerates. On the transfer path everything after this tick is
            // gated behind the regrant, which the ordered stream necessarily
            // delivers after the announcement.
            motion: (!installed && session.motion_granted).then(neutral_motion),
            gimbal: (!installed && session.lease_granted).then(neutral_gimbal),
            lease: None,
            motion_lease: release_motion.then_some(LeaseAction::Release),
            label: None,
            arm: false,
            disarm: false,
            arm_suppressed: false,
            disarm_suppressed: false,
            capture_active: false,
        }
    }

    fn install_after_seed(
        &mut self,
        candidate: CompiledProfile,
        now_ms: f64,
        releases_motion: bool,
    ) {
        self.active = Some(candidate);
        self.pending = None;
        self.activation_revision = self.activation_revision.wrapping_add(1);
        self.streaming = false;
        self.last_request_ms = NEVER_REQUESTED_MS;
        self.handover_releases_motion = false;
        if releases_motion {
            self.motion_phase = MotionPhase::Releasing;
            self.motion_action_ms = now_ms;
        } else {
            // The incoming physical map may expose deflection the outgoing map
            // could not observe at the installation boundary.
            self.mapping_neutral_pending = true;
        }
    }
}

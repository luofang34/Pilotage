//! The control runtime: the single stateful object that turns a raw device
//! sample plus session state into a [`ControlPlan`], and swaps profiles in
//! through a neutral, transactional handover.
//!
//! State that survives across ticks lives here — the active profile, the
//! session activation revision, the R3 baseline, the gimbal stream latch, the
//! lease-request debounce clock, and the arm/disarm edge baselines. A profile
//! reaches this runtime only through [`ControlRuntime::activate`], which
//! itself only accepts a [`CompiledProfile`]; invalid bytes never get here.

use crate::flight::{Capture, flight_axes};
use crate::plan::{
    AXIS_PITCH, AXIS_ROLL, AXIS_THROTTLE, AXIS_YAW, ActivationPlan, BUTTON_EDGE_PRESSED,
    ControlPlan, Frame, GIMBAL_NEUTRAL_BUTTON, LeaseAction,
};
use crate::profile::CompiledProfile;
use crate::quasimode::{
    LeasePlan, frame_plan, gimbal_demand, lease_plan, modifier_held, reset_edge, reset_held,
};
use crate::sample::{RawSample, SessionState};

/// Sentinel making the first lease request fire immediately (before any real
/// `now_ms`), rather than waiting out a debounce window from time zero.
const NEVER_REQUESTED_MS: f64 = f64::NEG_INFINITY;

/// The stateful web-control runtime. Construct it, then activate a compiled
/// profile before evaluating ticks.
#[derive(Debug, Default)]
pub struct ControlRuntime {
    active: Option<CompiledProfile>,
    pending: Option<CompiledProfile>,
    activation_revision: u32,
    reset_baseline: bool,
    streaming: bool,
    last_request_ms: f64,
    prev_arm: bool,
    prev_disarm: bool,
}

impl ControlRuntime {
    /// A runtime with no profile yet. The first [`Self::activate`] installs
    /// one immediately (there is nothing to neutrally hand over from).
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_request_ms: NEVER_REQUESTED_MS,
            ..Self::default()
        }
    }

    /// The current session activation revision (advances on each install).
    #[must_use]
    pub const fn activation_revision(&self) -> u32 {
        self.activation_revision
    }

    /// Whether a profile is live (false only before the first activation).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Begins activating a compiled profile. With no profile yet, installs it
    /// immediately. Otherwise opens a transaction: the current profile stays
    /// active while the shell emits neutral and releases the gimbal lease, and
    /// the candidate installs on the first tick that finds the captured
    /// controls neutral. Edge and quasimode state is cleared now, so a control
    /// held across the handover cannot fire a fresh edge on resume.
    pub fn activate(&mut self, candidate: CompiledProfile) -> ActivationPlan {
        self.streaming = false;
        if self.active.is_none() {
            self.install(candidate);
            return ActivationPlan {
                installed: true,
                activation_revision: self.activation_revision,
                emit_neutral: true,
                release_gimbal_lease: false,
            };
        }
        self.pending = Some(candidate);
        ActivationPlan {
            installed: false,
            activation_revision: self.activation_revision,
            emit_neutral: true,
            release_gimbal_lease: true,
        }
    }

    /// Installs a candidate as the active profile and advances the activation
    /// revision, re-seeding every edge baseline so nothing held fires.
    fn install(&mut self, candidate: CompiledProfile) {
        self.active = Some(candidate);
        self.pending = None;
        self.activation_revision = self.activation_revision.wrapping_add(1);
        self.reset_baseline = false;
        self.streaming = false;
        self.last_request_ms = NEVER_REQUESTED_MS;
        self.prev_arm = false;
        self.prev_disarm = false;
    }

    /// Evaluates one control tick. A dead session emits nothing; a pending
    /// activation emits the neutral handover until captured controls are
    /// neutral; otherwise the active profile drives flight and the gimbal
    /// quasimode.
    pub fn evaluate(&mut self, sample: &RawSample, session: &SessionState) -> ControlPlan {
        if !session.connected {
            return ControlPlan::default();
        }
        let Some(active) = self.active.clone() else {
            return ControlPlan::default();
        };
        if self.pending.is_some() {
            return self.evaluate_handover(sample, &active);
        }
        self.evaluate_active(sample, session, &active)
    }

    /// Emits the neutral handover, and installs the pending profile once the
    /// captured controls (the modifier and the gimbal sticks) read neutral, so
    /// behavior changes only after a genuine neutral transition.
    fn evaluate_handover(&mut self, sample: &RawSample, active: &CompiledProfile) -> ControlPlan {
        let captured_neutral = !modifier_held(sample, &active.gimbal) && {
            let demand = gimbal_demand(sample, &active.gimbal);
            demand.pitch == 0.0 && demand.yaw == 0.0
        };
        if let Some(candidate) = captured_neutral.then(|| self.pending.take()).flatten() {
            self.reset_baseline = reset_held(sample, &candidate.gimbal);
            self.prev_arm = sample.pressed(usize::from(candidate.flight.arm_button));
            self.prev_disarm = sample.pressed(usize::from(candidate.flight.disarm_button));
            self.install_after_seed(candidate);
        }
        ControlPlan {
            motion: Some(neutral_motion()),
            gimbal: Some(neutral_gimbal()),
            lease: Some(LeaseAction::Release),
            label: None,
        }
    }

    /// Installs after the edge baselines have been seeded to the current held
    /// state (so a held control does not fire), without re-clearing them.
    fn install_after_seed(&mut self, candidate: CompiledProfile) {
        self.active = Some(candidate);
        self.pending = None;
        self.activation_revision = self.activation_revision.wrapping_add(1);
        self.streaming = false;
        self.last_request_ms = NEVER_REQUESTED_MS;
    }

    /// The normal tick: flight motion (masked while LT is held), the gimbal
    /// quasimode frame, and the lease action.
    fn evaluate_active(
        &mut self,
        sample: &RawSample,
        session: &SessionState,
        active: &CompiledProfile,
    ) -> ControlPlan {
        let gimbal = &active.gimbal;
        let held = modifier_held(sample, gimbal);
        let active_gimbal = session.lease_granted && session.mode.carries_gimbal();

        let reset = reset_edge(
            reset_held(sample, gimbal),
            self.reset_baseline,
            active_gimbal,
        );
        self.reset_baseline = reset.baseline;
        let demand = gimbal_demand(sample, gimbal);
        let plan = frame_plan(held && active_gimbal, reset.edge, self.streaming, demand);
        self.streaming = plan.is_some_and(|p| p.streaming);

        let gimbal_frame = active_gimbal.then(|| {
            let (pitch, yaw, recenter) =
                plan.map_or((0.0, 0.0, false), |p| (p.pitch, p.yaw, p.recenter));
            let edges = if recenter {
                vec![(GIMBAL_NEUTRAL_BUTTON, BUTTON_EDGE_PRESSED)]
            } else {
                Vec::new()
            };
            Frame {
                axes: vec![(AXIS_PITCH, pitch), (AXIS_YAW, yaw)],
                edges,
            }
        });

        let motion = self.motion_frame(sample, session, active, held);
        let lease = self.lease_action(session);
        ControlPlan {
            motion: Some(motion.0),
            gimbal: gimbal_frame,
            lease,
            label: Some(motion.1),
        }
    }

    /// Builds the flight motion frame (masking the captured inputs while LT is
    /// held) with arm/disarm edges, and returns it with its readout label.
    fn motion_frame(
        &mut self,
        sample: &RawSample,
        session: &SessionState,
        active: &CompiledProfile,
        held: bool,
    ) -> (Frame, &'static str) {
        let flight = &active.flight;
        let capture = Capture {
            active: held,
            pitch_axis: active.gimbal.pitch.source_index,
            yaw_axis: active.gimbal.yaw.source_index,
            modifier_button: usize::from(active.gimbal.modifier_button),
        };
        let axes = flight_axes(sample, flight, session.mode, capture);
        let mut edges = Vec::new();
        self.push_edge(&mut edges, sample, flight.arm_button, true);
        self.push_edge(&mut edges, sample, flight.disarm_button, false);
        (
            Frame {
                axes: vec![
                    (AXIS_ROLL, axes.roll),
                    (AXIS_PITCH, axes.pitch),
                    (AXIS_THROTTLE, axes.throttle),
                    (AXIS_YAW, axes.yaw),
                ],
                edges,
            },
            axes.label,
        )
    }

    /// Records an arm (`arm == true`) or disarm rising edge and advances its
    /// baseline. Unrelated buttons pass through untouched by the quasimode.
    fn push_edge(&mut self, edges: &mut Vec<(u16, u8)>, sample: &RawSample, button: u8, arm: bool) {
        let pressed = sample.pressed(usize::from(button));
        let prev = if arm { self.prev_arm } else { self.prev_disarm };
        if pressed && !prev {
            edges.push((u16::from(button), BUTTON_EDGE_PRESSED));
        }
        if arm {
            self.prev_arm = pressed;
        } else {
            self.prev_disarm = pressed;
        }
    }

    /// Translates the lease plan into a [`LeaseAction`], recording the request
    /// time so repeated requests debounce.
    fn lease_action(&mut self, session: &SessionState) -> Option<LeaseAction> {
        match lease_plan(session, self.last_request_ms) {
            LeasePlan::Request => {
                self.last_request_ms = session.now_ms;
                Some(LeaseAction::Request)
            }
            LeasePlan::Release => Some(LeaseAction::Release),
            LeasePlan::None => None,
        }
    }
}

/// An explicit all-zero motion frame for the neutral handover.
fn neutral_motion() -> Frame {
    Frame {
        axes: vec![
            (AXIS_ROLL, 0.0),
            (AXIS_PITCH, 0.0),
            (AXIS_THROTTLE, 0.0),
            (AXIS_YAW, 0.0),
        ],
        edges: Vec::new(),
    }
}

/// An explicit zero-rate gimbal frame for the neutral handover.
fn neutral_gimbal() -> Frame {
    Frame {
        axes: vec![(AXIS_PITCH, 0.0), (AXIS_YAW, 0.0)],
        edges: Vec::new(),
    }
}

#[cfg(test)]
mod tests;

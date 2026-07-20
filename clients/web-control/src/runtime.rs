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
    // The session generation last evaluated. When the shell reports a NEW
    // generation (a fresh connect), the first tick seeds every discrete-action
    // baseline from the held state and fires no edge — a button held across a
    // disconnect/reconnect cannot become a fresh arm/disarm/recenter.
    last_generation: Option<u32>,
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

    /// The active profile's identity string, or empty before activation.
    #[must_use]
    pub fn active_profile_id(&self) -> &str {
        self.active.as_ref().map_or("", CompiledProfile::id)
    }

    /// The active profile's content digest, or all-zero before activation. The
    /// shell exposes this so a host can later bind the activation revision it
    /// sees on the wire to the exact profile bytes that produced it.
    #[must_use]
    pub fn active_profile_digest(&self) -> [u8; 32] {
        self.active
            .as_ref()
            .map_or([0u8; 32], CompiledProfile::digest)
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
        // Move the active profile out for the tick and restore it after, so the
        // hot path borrows it without cloning; a handover that installs a new
        // one replaces it, so we only restore when nothing was installed.
        let Some(active) = self.active.take() else {
            return ControlPlan::default();
        };
        self.prime_for_generation(sample, session, &active);
        let plan = if self.pending.is_some() {
            self.evaluate_handover(sample, &active)
        } else {
            self.evaluate_active(sample, session, &active)
        };
        if self.active.is_none() {
            self.active = Some(active);
        }
        plan
    }

    /// On the first tick of a NEW session generation, seeds every discrete
    /// baseline to the currently held state, so a control held across a
    /// disconnect/reconnect fires no edge. A no-op within a generation.
    fn prime_for_generation(
        &mut self,
        sample: &RawSample,
        session: &SessionState,
        active: &CompiledProfile,
    ) {
        if self.last_generation == Some(session.generation) {
            return;
        }
        self.last_generation = Some(session.generation);
        self.reset_baseline = reset_held(sample, &active.gimbal);
        self.prev_arm = sample.pressed(usize::from(active.flight.arm_button));
        self.prev_disarm = sample.pressed(usize::from(active.flight.disarm_button));
        self.streaming = false;
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
            arm: false,
            disarm: false,
            capture_active: false,
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
            motion: Some(motion.frame),
            gimbal: gimbal_frame,
            lease,
            label: Some(motion.label),
            arm: motion.arm,
            disarm: motion.disarm,
            capture_active: held && active_gimbal,
        }
    }

    /// Builds the flight motion frame (masking the captured inputs while the
    /// modifier is held) plus the TYPED arm/disarm edges and the readout label.
    /// Arm/disarm are typed, not physical button ids, so a rebound arm control
    /// cannot silently disable arming downstream.
    fn motion_frame(
        &mut self,
        sample: &RawSample,
        session: &SessionState,
        active: &CompiledProfile,
        held: bool,
    ) -> MotionOutcome {
        let flight = &active.flight;
        let capture = Capture {
            active: held,
            pitch_axis: active.gimbal.pitch.source_index,
            yaw_axis: active.gimbal.yaw.source_index,
            modifier_button: usize::from(active.gimbal.modifier_button),
        };
        let axes = flight_axes(sample, flight, session.mode, capture);
        MotionOutcome {
            frame: Frame {
                axes: vec![
                    (AXIS_ROLL, axes.roll),
                    (AXIS_PITCH, axes.pitch),
                    (AXIS_THROTTLE, axes.throttle),
                    (AXIS_YAW, axes.yaw),
                ],
                edges: Vec::new(),
            },
            label: axes.label,
            arm: self.edge_fired(sample, flight.arm_button, true),
            disarm: self.edge_fired(sample, flight.disarm_button, false),
        }
    }

    /// Whether an arm (`arm == true`) or disarm rising edge fired this tick,
    /// advancing its baseline. Unrelated buttons pass through untouched.
    fn edge_fired(&mut self, sample: &RawSample, button: u8, arm: bool) -> bool {
        let pressed = sample.pressed(usize::from(button));
        let prev = if arm { self.prev_arm } else { self.prev_disarm };
        if arm {
            self.prev_arm = pressed;
        } else {
            self.prev_disarm = pressed;
        }
        pressed && !prev
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

/// The motion tick outcome: the frame, its readout label, and the typed
/// arm/disarm edges (kept out of the frame so no physical index leaks).
struct MotionOutcome {
    frame: Frame,
    label: &'static str,
    arm: bool,
    disarm: bool,
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

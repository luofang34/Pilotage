//! The control runtime: the single stateful object that turns a raw device
//! sample plus session state into a [`ControlPlan`], and swaps profiles in
//! through a neutral, transactional handover.
//!
//! State that survives across ticks lives here — the active profile, the
//! session activation revision, the R3 baseline, the gimbal stream latch, the
//! lease-request debounce clock, and the arm/disarm edge baselines. A profile
//! reaches this runtime only through [`ControlRuntime::activate`], which
//! itself only accepts a [`CompiledProfile`]; invalid bytes never get here.

use crate::authority::{
    AuthorityDisposition, AuthorityEvent, AuthorityScope, AuthorityState, AuthorityTable,
};
use crate::flight::{Capture, flight_axes, shaped_stick};
use crate::plan::{ControlPlan, Frame, LeaseAction};
use crate::profile::CompiledProfile;
use crate::quasimode::{frame_plan, gimbal_demand, modifier_held, reset_edge, reset_held};
use crate::sample::{RawSample, SessionState};

mod activation;
mod authority;
use authority::{MotionOutput, MotionPhase};

/// The stateful web-control runtime. Construct it, then activate a compiled
/// profile before evaluating ticks.
#[derive(Debug, Default)]
pub struct ControlRuntime {
    active: Option<CompiledProfile>,
    pending: Option<CompiledProfile>,
    activation_revision: u32,
    reset_baseline: bool,
    streaming: bool,
    prev_arm: bool,
    prev_disarm: bool,
    // Where the motion lease sits during a real scope-member transfer. A
    // same-scope mapping activation retains authority and uses
    // `mapping_neutral_pending` instead.
    motion_phase: MotionPhase,
    // Every scope's reliable-stream authority and lease planner. JS feeds
    // events and executes returned actions; it holds no grant/fence policy.
    authority: AuthorityTable,
    // Set when the DEVICE mapping changed (a pad swap or re-selection): the
    // next tick re-seeds the discrete edge baselines from the held state, so
    // a button already pressed on the newly mapped device cannot fire as a
    // fresh arm/disarm/recenter. Unlike a generation change this does NOT
    // touch the motion-authority phase — a terminal denial stays terminal.
    reseed_edges: bool,
    // A same-scope activation retains its lease, but live output stays gated
    // until one neutral tick has been evaluated through the installed mapping.
    // This covers a non-standard incoming device map whose deflection is not
    // visible through the outgoing map used to decide the install boundary.
    mapping_neutral_pending: bool,
    // A pending activation releases motion only for a real scope-member
    // transfer. Mapping and scheme changes remain on the held generation.
    handover_releases_motion: bool,
}

impl ControlRuntime {
    /// A runtime with no profile yet. The first [`Self::activate`] installs
    /// one immediately (there is nothing to neutrally hand over from).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a fresh transport session, clearing every scope slot and
    /// re-seeding discrete controls on the next tick.
    pub fn begin_session(&mut self) {
        self.authority.begin_session();
        self.motion_phase = MotionPhase::Held;
        self.reseed_edges = true;
        self.streaming = false;
        self.handover_releases_motion = false;
        self.mapping_neutral_pending = false;
    }

    /// Re-seeds discrete controls before a datagram run starts on a live
    /// session. An unrecovered regrant enters neutral activation recovery.
    pub fn begin_control_run(&mut self) {
        self.reseed_edges = true;
        self.streaming = false;
        let motion = self.authority.state(AuthorityScope::Motion);
        self.motion_phase = if motion.denied() {
            MotionPhase::Denied
        } else if motion.recovered() {
            MotionPhase::Held
        } else {
            MotionPhase::Neutralizing
        };
    }

    /// Applies one filtered reliable-stream authority event.
    pub fn authority_event(
        &mut self,
        scope: AuthorityScope,
        event: AuthorityEvent,
    ) -> AuthorityDisposition {
        self.authority.apply(scope, event)
    }

    /// Returns the authoritative state of one scope.
    #[must_use]
    pub fn authority_state(&self, scope: AuthorityScope) -> AuthorityState {
        self.authority.state(scope)
    }

    /// Plans an explicit lease intent from orchestration outside the tick loop.
    pub fn plan_authority(
        &mut self,
        scope: AuthorityScope,
        desired: bool,
        now_ms: f64,
    ) -> Option<LeaseAction> {
        self.authority.plan_explicit(scope, desired, now_ms)
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

    /// Marks the discrete edge baselines for a re-seed on the next tick —
    /// called when the DEVICE mapping changes, so an input already held on
    /// the newly mapped device cannot fire as a fresh edge.
    pub fn reseed_edge_baselines(&mut self) {
        self.reseed_edges = true;
    }

    /// The active profile's identity string, or empty before activation.
    #[must_use]
    pub fn active_profile_id(&self) -> &str {
        self.active.as_ref().map_or("", CompiledProfile::id)
    }

    /// The active profile DOCUMENT revision — the ADR-0007/0009 device-profile
    /// revision carried on control frames, distinct from the session
    /// activation epoch ([`Self::activation_revision`]). Zero before activation.
    #[must_use]
    pub fn active_profile_revision(&self) -> u32 {
        self.active.as_ref().map_or(0, CompiledProfile::revision)
    }

    /// The active scheme's flight arm and disarm button slots, or `None`
    /// before activation — the slots operator-facing hints resolve through
    /// the device stage, so a rebound arm control renames its own hint.
    #[must_use]
    pub fn active_flight_buttons(&self) -> Option<(u8, u8)> {
        self.active
            .as_ref()
            .map(|active| (active.flight.arm_button, active.flight.disarm_button))
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
        if self.reseed_edges {
            self.reseed_edges = false;
            self.reset_baseline = reset_held(sample, &active.gimbal);
            self.prev_arm = sample.pressed(usize::from(active.flight.arm_button));
            self.prev_disarm = sample.pressed(usize::from(active.flight.disarm_button));
        }
        if session.input_lost {
            let plan = self.consume_lost_input(sample, &active);
            self.active = Some(active);
            return plan;
        }
        let plan = if self.pending.is_some() {
            self.evaluate_handover(sample, session, &active)
        } else {
            self.evaluate_active(sample, session, &active)
        };
        if self.active.is_none() {
            self.active = Some(active);
        }
        plan
    }

    fn consume_lost_input(&mut self, sample: &RawSample, active: &CompiledProfile) -> ControlPlan {
        self.reset_baseline = reset_held(sample, &active.gimbal);
        let arm = self.edge_fired(sample, active.flight.arm_button, true);
        let disarm = self.edge_fired(sample, active.flight.disarm_button, false);
        ControlPlan {
            arm_suppressed: arm,
            disarm_suppressed: disarm,
            ..ControlPlan::default()
        }
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
        let active_gimbal =
            self.authority.state(AuthorityScope::Gimbal).granted() && session.mode.carries_gimbal();
        // ONE effective capture condition drives masking, gimbal routing, and
        // the HUD: the modifier only captures when the gimbal is actually
        // active (a lease is held). Otherwise LT would silently suppress flight
        // while producing no gimbal output — a silent control loss.
        let captured = modifier_held(sample, gimbal) && active_gimbal;

        let reset = reset_edge(
            reset_held(sample, gimbal),
            self.reset_baseline,
            active_gimbal,
        );
        self.reset_baseline = reset.baseline;
        let demand = gimbal_demand(sample, gimbal);
        let plan = frame_plan(captured, reset.edge, self.streaming, demand);
        self.streaming = plan.is_some_and(|p| p.streaming);

        let gimbal_frame = active_gimbal.then(|| {
            let (pitch, yaw, recenter) =
                plan.map_or((0.0, 0.0, false), |p| (p.pitch, p.yaw, p.recenter));
            Frame::gimbal(pitch, yaw, recenter)
        });

        let outcome = self.motion_frame(sample, session, active, captured);
        let lease = self.authority.plan(
            AuthorityScope::Gimbal,
            session.mode.carries_gimbal(),
            session.now_ms,
        );
        let (motion_lease, authority_output) =
            self.advance_motion_authority(sample, session, active);
        let output = self.apply_mapping_neutral_gate(sample, active, authority_output);
        // Live output only under a held, granted lease; the recovery path emits
        // explicit neutral activation frames instead, and everything else gates.
        // Arm/disarm fire only when live (their baselines still advance while
        // gated, so a control held through recovery fires nothing on resume).
        let live = output == MotionOutput::Live;
        let motion = match output {
            MotionOutput::Gated => None,
            MotionOutput::Neutral => Some(neutral_motion()),
            MotionOutput::Live => Some(outcome.frame),
        };
        ControlPlan {
            motion,
            gimbal: gimbal_frame,
            lease,
            motion_lease,
            label: Some(outcome.label),
            arm: live && outcome.arm,
            disarm: live && outcome.disarm,
            // A press while gated is consumed (the edge baseline advanced), so
            // report it: the shell owes the operator an explanation for a
            // safety press that fired nothing.
            arm_suppressed: !live && outcome.arm,
            disarm_suppressed: !live && outcome.disarm,
            capture_active: captured,
        }
    }

    /// Holds a same-scope activation behind the installed mapping's own
    /// neutral sample: a deflection visible only through the incoming map
    /// must center once before anything publishes. Authority is never
    /// released on this path, so this is client-side hygiene, not host
    /// recovery — the single neutral emitted on satisfaction is a
    /// conservative first frame under the advanced revision, not a recovery
    /// proof. A gated authority cannot satisfy the gate: its tick publishes
    /// nothing, so nothing has demonstrated the centered state downstream.
    fn apply_mapping_neutral_gate(
        &mut self,
        sample: &RawSample,
        active: &CompiledProfile,
        authority_output: MotionOutput,
    ) -> MotionOutput {
        if !self.mapping_neutral_pending {
            return authority_output;
        }
        if !controls_neutral(sample, active) {
            return MotionOutput::Gated;
        }
        match authority_output {
            MotionOutput::Gated => MotionOutput::Gated,
            MotionOutput::Neutral => {
                self.mapping_neutral_pending = false;
                MotionOutput::Neutral
            }
            MotionOutput::Live => {
                self.mapping_neutral_pending = false;
                MotionOutput::Neutral
            }
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
        let axes = flight_axes(sample, flight, &active.flight_stick, session.mode, capture);
        MotionOutcome {
            frame: Frame::motion(axes.roll, axes.pitch, axes.throttle, axes.yaw),
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
}

/// A trigger reads neutral below this analog travel.
const TRIGGER_NEUTRAL_EPS: f32 = 0.05;

/// Whether every control-relevant input of `profile` reads neutral: the gimbal
/// modifier and axes, the four flight sticks, and the two triggers. Activation
/// requires this across BOTH the active and candidate profiles, so a profile
/// swap can never change a deflected input's meaning at the moment of install.
fn controls_neutral(sample: &RawSample, profile: &CompiledProfile) -> bool {
    if modifier_held(sample, &profile.gimbal) {
        return false;
    }
    let demand = gimbal_demand(sample, &profile.gimbal);
    if demand.pitch != 0.0 || demand.yaw != 0.0 {
        return false;
    }
    let flight = &profile.flight;
    let sticks = [flight.left_x, flight.left_y, flight.right_x, flight.right_y];
    if sticks
        .into_iter()
        .any(|index| shaped_stick(sample, &profile.flight_stick, index) != 0.0)
    {
        return false;
    }
    [flight.trigger_left, flight.trigger_right]
        .into_iter()
        .all(|index| sample.button_value(index) <= TRIGGER_NEUTRAL_EPS)
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
    Frame::motion(0.0, 0.0, 0.0, 0.0)
}

/// An explicit zero-rate gimbal frame for the neutral handover.
fn neutral_gimbal() -> Frame {
    Frame::gimbal(0.0, 0.0, false)
}

#[cfg(test)]
mod tests;

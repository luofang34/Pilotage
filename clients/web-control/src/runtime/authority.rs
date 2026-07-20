//! The motion-lease reacquisition state machine. On a live profile switch the
//! runtime fences the motion generation and recovers only through a neutral
//! activation burst, so the host clears its link-loss brake before a live
//! command resumes and nothing publishes on the released generation.

use crate::plan::LeaseAction;
use crate::profile::CompiledProfile;
use crate::sample::{RawSample, SessionState};

use super::{ControlRuntime, controls_neutral};

/// The motion lease's position in the reacquisition handshake after a profile
/// switch. Steady flight sits in [`MotionPhase::Held`]; a handover walks
/// `Held → Releasing → Reacquiring → Neutralizing → Held`, gating live motion
/// output the whole way and recovering only through a neutral activation burst
/// so the host clears its link-loss brake before a live command resumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum MotionPhase {
    /// Authority held on the current generation; motion frames publish live.
    #[default]
    Held,
    /// The handover released the motion lease; awaiting the session to reflect
    /// the release (`motion_granted` false) before re-requesting.
    Releasing,
    /// Re-requested the lease; awaiting a fresh grant (the shell verifies the
    /// grant's generation exceeds the release fence) or a terminal denial.
    /// Live output stays gated so nothing publishes on the released generation.
    Reacquiring,
    /// Regranted on a fresh generation: gate live output and, once the
    /// operator's controls read neutral, transmit a burst of neutral activation
    /// frames under the fresh generation so the host clears its link-loss brake.
    /// Only then does live output resume — with the operator already neutral, so
    /// no command jumps at recovery.
    Neutralizing,
    /// The reacquire was denied: terminal. Live output stays gated with no
    /// further requests until a fresh session (a reconnect re-primes to `Held`).
    Denied,
}

/// What the runtime does with motion output this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MotionOutput {
    /// Send no motion frame (reacquiring, denied, or the operator is deflected
    /// mid-recovery): nothing rides the released or unrecovered generation.
    Gated,
    /// Send an explicit neutral frame under the fresh generation — the recovery
    /// activation the host needs to clear its link-loss brake.
    Neutral,
    /// Publish the live mapped frame.
    Live,
}

/// A dropped motion release/request is re-emitted after this long without the
/// session reflecting the expected transition.
const MOTION_LEASE_RETRY_MS: f64 = 250.0;

/// Consecutive neutral activation frames transmitted (once the operator is
/// neutral) before live output resumes. A burst, not a single datagram, so the
/// host's link-loss recovery survives packet loss on the control channel.
const NEUTRAL_ACTIVATION_FRAMES: u8 = 3;

impl ControlRuntime {
    /// Advances the motion-lease reacquisition after a profile handover and
    /// returns any lease action to emit plus what to do with motion output. The
    /// full handshake is
    /// `release → (session reflects release) → request → (fresh grant) →
    /// (operator neutral) → neutral activation burst → live`:
    ///
    /// - the request is never emitted until the session confirms the release,
    ///   and a dropped release/request is re-emitted after
    ///   [`MOTION_LEASE_RETRY_MS`] so a lost write cannot wedge the handshake;
    /// - a denied reacquire is terminal — no more requests;
    /// - on a fresh grant, live output stays gated until the operator's controls
    ///   read neutral, then the runtime transmits [`NEUTRAL_ACTIVATION_FRAMES`]
    ///   neutral frames under the fresh generation (the activation the host
    ///   needs to clear its link-loss brake) before resuming live, so no live
    ///   command ever rides the released generation or jumps at recovery.
    pub(super) fn advance_motion_authority(
        &mut self,
        sample: &RawSample,
        session: &SessionState,
        active: &CompiledProfile,
    ) -> (Option<LeaseAction>, MotionOutput) {
        let stalled = session.now_ms - self.motion_action_ms >= MOTION_LEASE_RETRY_MS;
        match self.motion_phase {
            // Belt-and-suspenders: even Held never publishes live without a
            // current grant, so a motion frame can never ride an ungranted lease.
            MotionPhase::Held if session.motion_granted => (None, MotionOutput::Live),
            MotionPhase::Held | MotionPhase::Denied => (None, MotionOutput::Gated),
            MotionPhase::Releasing => {
                if !session.motion_granted {
                    // The host has released; only now request the new grant.
                    self.motion_phase = MotionPhase::Reacquiring;
                    self.motion_action_ms = session.now_ms;
                    (Some(LeaseAction::Request), MotionOutput::Gated)
                } else if stalled {
                    self.motion_action_ms = session.now_ms;
                    (Some(LeaseAction::Release), MotionOutput::Gated)
                } else {
                    (None, MotionOutput::Gated)
                }
            }
            MotionPhase::Reacquiring => {
                if session.motion_denied {
                    // A denied reacquire is terminal: stop requesting.
                    self.motion_phase = MotionPhase::Denied;
                    (None, MotionOutput::Gated)
                } else if session.motion_granted {
                    // Fresh grant (the shell verified the generation exceeds the
                    // fence): begin the neutral-activation recovery.
                    self.motion_phase = MotionPhase::Neutralizing;
                    self.motion_neutral_frames = 0;
                    (None, MotionOutput::Gated)
                } else if stalled {
                    self.motion_action_ms = session.now_ms;
                    (Some(LeaseAction::Request), MotionOutput::Gated)
                } else {
                    (None, MotionOutput::Gated)
                }
            }
            MotionPhase::Neutralizing => {
                if controls_neutral(sample, active) {
                    // Transmit neutral activation frames under the fresh
                    // generation; a burst rides out control-channel packet loss
                    // before live output resumes.
                    self.motion_neutral_frames = self.motion_neutral_frames.saturating_add(1);
                    if self.motion_neutral_frames >= NEUTRAL_ACTIVATION_FRAMES {
                        self.motion_phase = MotionPhase::Held;
                    }
                    (None, MotionOutput::Neutral)
                } else {
                    // The operator is still commanding: never resume onto a live
                    // deflection and never count a non-neutral frame. Wait.
                    self.motion_neutral_frames = 0;
                    (None, MotionOutput::Gated)
                }
            }
        }
    }
}

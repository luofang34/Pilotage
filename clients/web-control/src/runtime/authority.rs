//! The motion-lease reacquisition state machine. On a scope-member transfer the
//! runtime fences the motion generation and recovers only through a neutral
//! activation burst, so the host clears its link-loss brake before a live
//! command resumes and nothing publishes on the released generation.

use crate::authority::AuthorityScope;
use crate::plan::LeaseAction;
use crate::profile::CompiledProfile;
use crate::sample::{RawSample, SessionState};

use super::{ControlRuntime, controls_neutral};

/// The motion lease's position in the reacquisition handshake after a scope
/// transfer. Steady flight sits in [`MotionPhase::Held`]; a handover walks
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

impl ControlRuntime {
    /// Advances motion-lease reacquisition after a scope-member transfer and
    /// returns any lease action to emit plus what to do with motion output. The
    /// full handshake is
    /// `release → (session reflects release) → request → (fresh grant) →
    /// (operator neutral) → neutral activation (retransmitted) → (host confirms
    /// recovery) → live`:
    ///
    /// - the request is never emitted until the session confirms the release,
    ///   and a dropped release/request is re-emitted after
    ///   [`MOTION_LEASE_RETRY_MS`] so a lost write cannot wedge the handshake;
    /// - a denied reacquire is terminal — no more requests;
    /// - on a fresh grant, live output stays gated until the operator's controls
    ///   read neutral, then the runtime transmits neutral frames under the fresh
    ///   generation EVERY tick (retransmitted, so a dropped datagram cannot
    ///   wedge recovery) and stays neutralizing until the host CONFIRMS it
    ///   cleared the vehicle's link-loss latch (`motion_recovered`). Only then
    ///   does live resume — never on the released generation, never onto a
    ///   deflection, and never on the mere hope that a best-effort datagram
    ///   arrived.
    pub(super) fn advance_motion_authority(
        &mut self,
        sample: &RawSample,
        session: &SessionState,
        active: &CompiledProfile,
    ) -> (Option<LeaseAction>, MotionOutput) {
        let motion = self.authority.state(AuthorityScope::Motion);
        match self.motion_phase {
            // Belt-and-suspenders: even Held never publishes live without a
            // current grant, so a motion frame can never ride an ungranted lease.
            MotionPhase::Held if motion.granted() => (None, MotionOutput::Live),
            MotionPhase::Held if motion.denied() => {
                self.motion_phase = MotionPhase::Denied;
                (None, MotionOutput::Gated)
            }
            MotionPhase::Held => {
                self.motion_phase = MotionPhase::Reacquiring;
                let action = self
                    .authority
                    .plan(AuthorityScope::Motion, true, session.now_ms);
                (action, MotionOutput::Gated)
            }
            MotionPhase::Denied => (None, MotionOutput::Gated),
            MotionPhase::Releasing => {
                if !motion.granted() {
                    // The host has released; only now request the new grant.
                    self.motion_phase = MotionPhase::Reacquiring;
                    let action = self
                        .authority
                        .plan(AuthorityScope::Motion, true, session.now_ms);
                    (action, MotionOutput::Gated)
                } else {
                    let action = self
                        .authority
                        .plan(AuthorityScope::Motion, false, session.now_ms);
                    (action, MotionOutput::Gated)
                }
            }
            MotionPhase::Reacquiring => {
                if motion.denied() {
                    // A denied reacquire is terminal: stop requesting.
                    self.motion_phase = MotionPhase::Denied;
                    (None, MotionOutput::Gated)
                } else if motion.granted() {
                    // Fresh grant (the shell verified the generation exceeds the
                    // fence): begin the neutral-activation recovery.
                    self.motion_phase = MotionPhase::Neutralizing;
                    (None, MotionOutput::Gated)
                } else {
                    let action = self
                        .authority
                        .plan(AuthorityScope::Motion, true, session.now_ms);
                    (action, MotionOutput::Gated)
                }
            }
            MotionPhase::Neutralizing => {
                if motion.recovered() {
                    // The host CONFIRMED it cleared the vehicle's link-loss latch
                    // on this fresh generation: recovery is complete. One final
                    // neutral this tick, then live resumes.
                    self.motion_phase = MotionPhase::Held;
                    (None, MotionOutput::Neutral)
                } else if controls_neutral(sample, active) {
                    // Transmit a neutral activation frame under the fresh
                    // generation, EVERY tick, so a dropped datagram is
                    // retransmitted until the host confirms recovery.
                    (None, MotionOutput::Neutral)
                } else {
                    // The operator is still commanding: never resume onto a live
                    // deflection. Wait for physical neutral before activating.
                    (None, MotionOutput::Gated)
                }
            }
        }
    }
}

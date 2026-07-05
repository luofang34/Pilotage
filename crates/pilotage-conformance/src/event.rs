//! The ordered observable-event log a [`ScriptedSession`] records
//! (ADR-0012).
//!
//! Every script step appends zero or more [`SessionEvent`]s in the order the
//! embedding host would observe and persist them: authority effects, control
//! frame verdicts and their adapter dispositions, and adapter step outcomes.
//! The log is the determinism oracle — replaying the same script from the
//! same seed produces a byte-for-byte equal `Vec<SessionEvent>` — and the
//! authority-effect subsequence is the exact audit trail ADR-0012 requires.
//!
//! [`ScriptedSession`]: crate::ScriptedSession

use pilotage_adapter_api::Disposition;
use pilotage_authority::{AuthorityEffect, FrameVerdict};
use pilotage_protocol::{SequenceNum, VehicleId};
use pilotage_timing::SimTick;

/// The combined authority-then-adapter outcome of routing one control frame
/// through the session.
///
/// A frame is verified against authority first; only an
/// [`FrameVerdict::Accepted`] frame is applied to the adapter, so the
/// `disposition` is present exactly when the verdict accepted the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameOutcome {
    /// Sequence number of the frame, for correlating against the script.
    pub sequence: SequenceNum,
    /// Authority verdict for the frame's `(scope, generation)`.
    pub verdict: FrameVerdict,
    /// Adapter disposition, present only when authority accepted the frame.
    pub disposition: Option<Disposition>,
    /// Simulation tick the disposition corresponds to, when applied.
    pub applied_tick: Option<SimTick>,
}

/// One observable event in a scripted session's ordered log (ADR-0012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// An authority engine effect (the ADR-0012 authority audit event).
    Authority(AuthorityEffect),
    /// A control frame was routed through authority and, if accepted, the
    /// adapter.
    Frame(FrameOutcome),
    /// The adapter advanced by `advanced` ticks to simulation tick `now`.
    Stepped {
        /// Ticks actually advanced.
        advanced: u32,
        /// Simulation tick after advancing.
        now: SimTick,
    },
    /// A link-loss policy was engaged on the adapter for `vehicle` (`None`
    /// signals link recovery).
    LinkLossPolicyEngaged {
        /// Vehicle the policy was set on.
        vehicle: VehicleId,
        /// Human-readable policy label, kept string-typed so the event log
        /// carries no adapter-crate type that lacks `Eq`.
        policy: &'static str,
    },
}

impl SessionEvent {
    /// Returns the wrapped authority effect when this event is an authority
    /// effect, for extracting the audit subsequence.
    #[must_use]
    pub fn as_authority(&self) -> Option<&AuthorityEffect> {
        match self {
            SessionEvent::Authority(effect) => Some(effect),
            _ => None,
        }
    }

    /// Returns the wrapped frame outcome when this event routed a control
    /// frame.
    #[must_use]
    pub fn as_frame(&self) -> Option<&FrameOutcome> {
        match self {
            SessionEvent::Frame(outcome) => Some(outcome),
            _ => None,
        }
    }
}

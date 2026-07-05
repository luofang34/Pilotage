//! Control-application outcomes and link-loss policy vocabulary (ADR-0008).

use pilotage_timing::SimTick;
use serde::{Deserialize, Serialize};

/// Why an adapter did not accept a control frame as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// The frame targeted a scope the vehicle does not expose.
    UnknownScope,
    /// The frame targeted a logical axis the scope does not accept.
    UnknownAxis,
    /// The frame targeted a vehicle the adapter does not know.
    UnknownVehicle,
    /// The frame failed a fencing check (stale generation or sequence).
    Fenced,
    /// The adapter rejected the frame for a reason not covered above.
    Other(String),
}

/// How an adapter disposed of an applied control frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// The frame was applied exactly as received.
    Accepted,
    /// The frame was applied after the adapter transformed it (e.g.
    /// clamping to a physical limit).
    Transformed,
    /// The frame was constrained by a safety or authority rule and only
    /// partially applied.
    Constrained,
    /// The frame was not applied.
    Rejected(RejectReason),
}

/// The result of applying a single control frame (ADR-0008): the simulation
/// tick the outcome corresponds to, and how the frame was disposed of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// Simulation tick this outcome corresponds to.
    pub tick: SimTick,
    /// How the frame was disposed of.
    pub disposition: Disposition,
}

/// What an adapter does to a vehicle when its control link is judged lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkLossPolicy {
    /// Zero all controls immediately.
    Neutralize,
    /// Apply maximum braking.
    Brake,
    /// Hold the last-known controls for a bounded number of ticks, then
    /// neutralize.
    HoldBrief {
        /// Ticks to hold the last-known controls before neutralizing.
        ticks: u32,
    },
    /// Freeze the vehicle in place.
    Pause,
    /// Hand control to an onboard automation system.
    EngageAutomation,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ApplyOutcome, Disposition, LinkLossPolicy, RejectReason};
    use pilotage_timing::SimTick;

    #[test]
    fn apply_outcome_holds_tick_and_disposition() {
        let outcome = ApplyOutcome {
            tick: SimTick::new(7),
            disposition: Disposition::Accepted,
        };
        assert_eq!(outcome.tick.as_u64(), 7);
        assert_eq!(outcome.disposition, Disposition::Accepted);
    }

    #[test]
    fn rejected_carries_reason() {
        let disposition = Disposition::Rejected(RejectReason::UnknownScope);
        assert_eq!(
            disposition,
            Disposition::Rejected(RejectReason::UnknownScope)
        );
    }

    #[test]
    fn hold_brief_carries_tick_count() {
        let policy = LinkLossPolicy::HoldBrief { ticks: 5 };
        assert_eq!(policy, LinkLossPolicy::HoldBrief { ticks: 5 });
    }
}

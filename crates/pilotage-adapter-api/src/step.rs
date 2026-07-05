//! Stepped-execution vocabulary for deterministic and accelerated adapters
//! (ADR-0008, ADR-0013).

use pilotage_timing::SimTick;

/// A request to advance an adapter by a bounded number of ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepBudget {
    /// Number of ticks the caller is requesting.
    pub ticks: u32,
}

/// How far an adapter actually advanced in response to a `StepBudget`.
///
/// `advanced` may be less than the requested budget (e.g. an adapter that
/// stops early on a terminal condition); callers must not assume the full
/// budget was consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    /// Number of ticks actually advanced.
    pub advanced: u32,
    /// Simulation tick after advancing.
    pub now: SimTick,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{StepBudget, StepOutcome};
    use pilotage_timing::SimTick;

    #[test]
    fn step_budget_holds_requested_ticks() {
        let budget = StepBudget { ticks: 10 };
        assert_eq!(budget.ticks, 10);
    }

    #[test]
    fn step_outcome_may_advance_less_than_requested() {
        let outcome = StepOutcome {
            advanced: 3,
            now: SimTick::new(3),
        };
        assert_eq!(outcome.advanced, 3);
        assert_eq!(outcome.now.as_u64(), 3);
    }
}

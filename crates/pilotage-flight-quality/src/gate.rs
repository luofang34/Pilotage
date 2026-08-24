use serde::{Deserialize, Serialize};

/// One hard trial gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardGate {
    /// The trial identity is valid and complete.
    TrialIdentity,
    /// The trial has no crash or unexpected contact.
    CrashOrUnexpectedContact,
    /// All required command, estimate, and truth values are finite.
    FiniteSignals,
    /// Position stays inside the declared bound.
    PositionBound,
    /// Attitude stays inside the declared bound.
    AttitudeBound,
    /// Body rate stays inside the declared bound.
    RateBound,
    /// Load stays inside the declared bound.
    LoadBound,
    /// Actuator saturation stays inside the declared duration bound.
    ActuatorSaturationDuration,
    /// The estimator stays valid.
    EstimatorValidity,
    /// The command link stays valid.
    CommandLinkValidity,
    /// The vehicle recovers before the phase deadline.
    RecoveryDeadline,
}

impl HardGate {
    /// The fixed hard-gate evaluation order for scorer version one.
    pub const ORDER: [Self; 11] = [
        Self::CrashOrUnexpectedContact,
        Self::TrialIdentity,
        Self::FiniteSignals,
        Self::PositionBound,
        Self::AttitudeBound,
        Self::RateBound,
        Self::LoadBound,
        Self::ActuatorSaturationDuration,
        Self::EstimatorValidity,
        Self::CommandLinkValidity,
        Self::RecoveryDeadline,
    ];
}

/// Typed context for one hard gate outcome.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum GateContext {
    /// The outcome applies to the complete trial.
    Trial,
    /// The outcome applies to one phase.
    Phase {
        /// The phase index.
        phase_index: usize,
    },
    /// The outcome applies to one sample.
    Sample {
        /// The phase index.
        phase_index: usize,
        /// The sample index.
        sample_index: usize,
        /// Trial time, in seconds.
        time_s: f64,
    },
    /// The outcome compares one observed value with one limit.
    Limit {
        /// The phase index.
        phase_index: usize,
        /// The sample index.
        sample_index: usize,
        /// Trial time, in seconds.
        time_s: f64,
        /// The observed value.
        observed: f64,
        /// The applicable limit.
        limit: f64,
    },
}

/// The result for one hard gate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardGateOutcome {
    /// The evaluated gate.
    pub gate: HardGate,
    /// Whether the trial passed this gate.
    pub passed: bool,
    /// The location or limit for this outcome.
    pub context: GateContext,
}

impl HardGateOutcome {
    /// Creates a passing gate outcome.
    #[must_use]
    pub const fn pass(gate: HardGate, context: GateContext) -> Self {
        Self {
            gate,
            passed: true,
            context,
        }
    }

    /// Creates a failing gate outcome.
    #[must_use]
    pub const fn fail(gate: HardGate, context: GateContext) -> Self {
        Self {
            gate,
            passed: false,
            context,
        }
    }
}

/// The ordered hard gate results for one trial.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardGateReport {
    outcomes: Vec<HardGateOutcome>,
}

impl HardGateReport {
    /// Creates a report from ordered gate outcomes.
    #[must_use]
    pub fn new(outcomes: Vec<HardGateOutcome>) -> Self {
        Self { outcomes }
    }

    /// Returns all gate outcomes in evaluation order.
    #[must_use]
    pub fn outcomes(&self) -> &[HardGateOutcome] {
        &self.outcomes
    }

    /// Returns true only when the report has outcomes and all gates pass.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.outcomes.len() == HardGate::ORDER.len()
            && self.ordered_prefix_is_valid()
            && self.outcomes.iter().all(|outcome| outcome.passed)
    }

    /// Returns true when outcomes are a nonempty prefix of the fixed order.
    #[must_use]
    pub fn ordered_prefix_is_valid(&self) -> bool {
        !self.outcomes.is_empty()
            && self.outcomes.len() <= HardGate::ORDER.len()
            && self
                .outcomes
                .iter()
                .zip(HardGate::ORDER)
                .all(|(outcome, required)| outcome.gate == required)
    }

    /// Returns each failed gate outcome.
    pub fn failures(&self) -> impl Iterator<Item = &HardGateOutcome> {
        self.outcomes.iter().filter(|outcome| !outcome.passed)
    }
}

#[cfg(test)]
mod tests;

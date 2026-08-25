use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{Digest, HardGateFailure, MetricValues, RunExecutionContext, RunRecord, TuneError};

use super::diagnostic::RunTerminalDiagnostic;
use super::digest::domain_digest;
use super::invalid_terminal;

/// The supported semantic terminal intent schema.
pub const RUN_TERMINAL_INTENT_SCHEMA_VERSION: u16 = 1;

const INTENT_DOMAIN: &[u8] = b"pilotage.flight-tune.run-terminal-intent.v1\0";

/// The semantic result that exists before terminal operations start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTerminalSemanticOutcome {
    /// The scenario and metric evaluation completed.
    ScenarioComplete {
        /// The candidate that produced the result.
        candidate_digest: Digest,
        /// The exact scenario artifact identity.
        scenario_digest: Digest,
        /// The exact passing run record.
        run: RunRecord,
    },
    /// A streaming hard gate stopped the run.
    HardGateAbort {
        /// The candidate that produced the failure.
        candidate_digest: Digest,
        /// The exact scenario artifact identity.
        scenario_digest: Digest,
        /// The exact first hard gate failure.
        failure: HardGateFailure,
    },
    /// Run execution failed before a semantic result was available.
    ExecutionError {
        /// The bounded execution failure identity.
        diagnostic: RunTerminalDiagnostic,
    },
    /// Recovery contains a run without a durable semantic result.
    Recovery,
}

impl RunTerminalSemanticOutcome {
    /// Reports whether successful containment can complete this result.
    #[must_use]
    pub const fn permits_completion(&self) -> bool {
        matches!(
            self,
            Self::ScenarioComplete { .. } | Self::HardGateAbort { .. }
        )
    }
}

/// One immutable semantic intent made before terminal operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTerminalIntent {
    schema_version: u16,
    context: RunExecutionContext,
    run_intent_digest: Digest,
    outcome: RunTerminalSemanticOutcome,
    intent_digest: Digest,
}

#[derive(Serialize)]
struct IntentDocument<'a> {
    schema_version: u16,
    context: &'a RunExecutionContext,
    run_intent_digest: Digest,
    outcome: &'a RunTerminalSemanticOutcome,
}

impl RunTerminalIntent {
    /// Creates a semantic intent for one exact run.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the outcome does not match the run or encoding fails.
    pub fn new(
        context: &RunExecutionContext,
        run_intent_digest: Digest,
        outcome: RunTerminalSemanticOutcome,
    ) -> Result<Self, TuneError> {
        let mut intent = Self {
            schema_version: RUN_TERMINAL_INTENT_SCHEMA_VERSION,
            context: context.clone(),
            run_intent_digest,
            outcome,
            intent_digest: Digest::from_bytes([0; 32]),
        };
        intent.intent_digest = intent.recompute_digest()?;
        Ok(intent)
    }

    /// Validates the run binding, semantic result, and canonical digest.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the intent is incomplete or changed.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.validate_content()?;
        if self.intent_digest.is_zero() || self.intent_digest != self.recompute_digest()? {
            return Err(invalid_terminal("the terminal intent digest changed"));
        }
        Ok(())
    }

    /// Recomputes the domain-separated intent digest.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the intent is invalid or encoding fails.
    pub fn recompute_digest(&self) -> Result<Digest, TuneError> {
        self.validate_content()?;
        domain_digest(
            INTENT_DOMAIN,
            &IntentDocument {
                schema_version: self.schema_version,
                context: &self.context,
                run_intent_digest: self.run_intent_digest,
                outcome: &self.outcome,
            },
            "run terminal intent",
        )
    }

    /// Returns the exact run context.
    #[must_use]
    pub const fn context(&self) -> &RunExecutionContext {
        &self.context
    }

    /// Returns the canonical run intent identity.
    #[must_use]
    pub const fn run_intent_digest(&self) -> Digest {
        self.run_intent_digest
    }

    /// Returns the semantic result.
    #[must_use]
    pub const fn outcome(&self) -> &RunTerminalSemanticOutcome {
        &self.outcome
    }

    /// Returns the canonical terminal intent identity.
    #[must_use]
    pub const fn intent_digest(&self) -> Digest {
        self.intent_digest
    }

    fn validate_content(&self) -> Result<(), TuneError> {
        self.context.validate()?;
        if self.schema_version != RUN_TERMINAL_INTENT_SCHEMA_VERSION
            || self.run_intent_digest.is_zero()
            || self.run_intent_digest != self.context.digest()?
        {
            return Err(invalid_terminal("the terminal intent run identity changed"));
        }
        validate_outcome(&self.outcome, &self.context)
    }
}

fn validate_outcome(
    outcome: &RunTerminalSemanticOutcome,
    context: &RunExecutionContext,
) -> Result<(), TuneError> {
    match outcome {
        RunTerminalSemanticOutcome::ScenarioComplete {
            candidate_digest,
            scenario_digest,
            run,
        } => validate_run(*candidate_digest, *scenario_digest, run, context),
        RunTerminalSemanticOutcome::HardGateAbort {
            candidate_digest,
            scenario_digest,
            failure,
        } => validate_failure(*candidate_digest, *scenario_digest, failure, context),
        RunTerminalSemanticOutcome::ExecutionError { diagnostic } => diagnostic.validate(),
        RunTerminalSemanticOutcome::Recovery => Ok(()),
    }
}

fn validate_run(
    candidate_digest: Digest,
    scenario_digest: Digest,
    run: &RunRecord,
    context: &RunExecutionContext,
) -> Result<(), TuneError> {
    validate_semantic_identity(candidate_digest, scenario_digest, context)?;
    if run.scenario_set != context.scenario_set()
        || run.scenario_id != context.scenario_id()
        || run.repetition != context.repetition()
        || run.seed != context.seed()
    {
        return Err(invalid_terminal(
            "the completed run record does not match its run context",
        ));
    }
    crate::score::validate_metric(&MetricValues {
        loss: run.loss,
        control_effort: run.control_effort,
        objectives: run.objectives.clone(),
    })?;
    validate_passed_gates(&run.passed_hard_gates)
}

fn validate_failure(
    candidate_digest: Digest,
    scenario_digest: Digest,
    failure: &HardGateFailure,
    context: &RunExecutionContext,
) -> Result<(), TuneError> {
    validate_semantic_identity(candidate_digest, scenario_digest, context)?;
    if failure.scenario_set != context.scenario_set()
        || failure.scenario_id != context.scenario_id()
        || failure.repetition != context.repetition()
        || failure.seed != context.seed()
        || failure.gate.passed
        || failure.gate.id.trim().is_empty()
        || failure.gate.detail.trim().is_empty()
    {
        return Err(invalid_terminal(
            "the hard gate failure does not match its run context",
        ));
    }
    Ok(())
}

fn validate_semantic_identity(
    candidate_digest: Digest,
    scenario_digest: Digest,
    context: &RunExecutionContext,
) -> Result<(), TuneError> {
    if candidate_digest != context.candidate_digest()
        || scenario_digest != context.scenario_digest()
    {
        return Err(invalid_terminal(
            "the semantic result artifact identity does not match its run context",
        ));
    }
    Ok(())
}

fn validate_passed_gates(gates: &[String]) -> Result<(), TuneError> {
    let mut unique = BTreeSet::new();
    if gates.is_empty()
        || gates.iter().any(|gate| {
            gate.trim().is_empty()
                || gate.chars().any(char::is_whitespace)
                || !unique.insert(gate.as_str())
        })
    {
        return Err(invalid_terminal(
            "the completed run hard gate set is empty or repeated",
        ));
    }
    Ok(())
}

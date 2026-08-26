use serde::{Deserialize, Serialize};

use crate::{Digest, TuneError};

use super::digest::digest_bytes;
use super::{RunTerminalIntent, RunTerminalReport, RunTerminalSemanticOutcome, invalid_terminal};

/// The supported terminal class schema.
pub const RUN_TERMINAL_CLASS_SCHEMA_VERSION: u16 = 1;

const CLASS_POLICY_DOMAIN: &[u8] = b"pilotage.flight-tune.run-terminal-class-policy.v1\0";

/// A completed semantic terminal class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTerminalCompletion {
    /// A complete scenario run passed terminal containment.
    ScenarioComplete,
    /// A hard gate abort passed terminal containment.
    HardGateAbort,
}

/// A stable quarantine terminal class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTerminalQuarantine {
    /// A required terminal operation failed.
    TerminalFailure,
    /// Run execution failed.
    ExecutionFailure,
    /// Recovery contained an interrupted run.
    Recovery,
    /// Completed evidence could not become durable.
    EvidenceFailure,
}

/// The one actual disposition of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTerminalDisposition {
    /// The run can contribute its semantic result.
    Completed {
        /// The completed result class.
        completion: RunTerminalCompletion,
    },
    /// The run cannot contribute evidence.
    Quarantine {
        /// The stable quarantine class.
        quarantine: RunTerminalQuarantine,
    },
}

/// The policy-derived actual class of one terminal report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTerminalClass {
    schema_version: u16,
    policy_digest: Digest,
    disposition: RunTerminalDisposition,
}

impl RunTerminalClass {
    /// Classifies one exact semantic intent and terminal report.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the report does not bind the intent.
    pub fn classify(
        intent: &RunTerminalIntent,
        report: &RunTerminalReport,
    ) -> Result<Self, TuneError> {
        validate_pair(intent, report)?;
        let disposition = base_disposition(intent, report);
        Ok(Self {
            schema_version: RUN_TERMINAL_CLASS_SCHEMA_VERSION,
            policy_digest: run_terminal_policy_digest(),
            disposition,
        })
    }

    /// Creates the quarantine class for a definite completed-evidence absence.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] unless the normal class is completed.
    pub fn evidence_failure(
        intent: &RunTerminalIntent,
        report: &RunTerminalReport,
    ) -> Result<Self, TuneError> {
        let base = Self::classify(intent, report)?;
        if !base.is_completed() {
            return Err(invalid_terminal(
                "evidence failure cannot replace an existing quarantine class",
            ));
        }
        Ok(Self {
            schema_version: RUN_TERMINAL_CLASS_SCHEMA_VERSION,
            policy_digest: run_terminal_policy_digest(),
            disposition: RunTerminalDisposition::Quarantine {
                quarantine: RunTerminalQuarantine::EvidenceFailure,
            },
        })
    }

    /// Validates this class for one exact intent and report.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the class or policy identity differs.
    pub fn validate_for(
        &self,
        intent: &RunTerminalIntent,
        report: &RunTerminalReport,
    ) -> Result<(), TuneError> {
        if self.schema_version != RUN_TERMINAL_CLASS_SCHEMA_VERSION
            || self.policy_digest != run_terminal_policy_digest()
        {
            return Err(invalid_terminal("the terminal class policy changed"));
        }
        let expected = match self.disposition {
            RunTerminalDisposition::Quarantine {
                quarantine: RunTerminalQuarantine::EvidenceFailure,
            } => Self::evidence_failure(intent, report)?,
            _ => Self::classify(intent, report)?,
        };
        if *self != expected {
            return Err(invalid_terminal(
                "the terminal class does not match its intent and report",
            ));
        }
        Ok(())
    }

    /// Returns the class policy identity.
    #[must_use]
    pub const fn policy_digest(self) -> Digest {
        self.policy_digest
    }

    /// Returns the actual terminal disposition.
    #[must_use]
    pub const fn disposition(self) -> RunTerminalDisposition {
        self.disposition
    }

    /// Reports whether this class can contribute a semantic result.
    #[must_use]
    pub const fn is_completed(self) -> bool {
        matches!(self.disposition, RunTerminalDisposition::Completed { .. })
    }
}

/// Returns the domain-separated terminal class policy identity.
#[must_use]
pub fn run_terminal_policy_digest() -> Digest {
    digest_bytes(CLASS_POLICY_DOMAIN)
}

fn validate_pair(intent: &RunTerminalIntent, report: &RunTerminalReport) -> Result<(), TuneError> {
    intent.validate()?;
    report.validate()?;
    if report.intent() != intent {
        return Err(invalid_terminal(
            "the terminal report does not bind the supplied intent",
        ));
    }
    Ok(())
}

fn base_disposition(
    intent: &RunTerminalIntent,
    report: &RunTerminalReport,
) -> RunTerminalDisposition {
    match intent.outcome() {
        RunTerminalSemanticOutcome::ScenarioComplete { .. } if report.all_required_succeeded() => {
            completed(RunTerminalCompletion::ScenarioComplete)
        }
        RunTerminalSemanticOutcome::HardGateAbort { .. } if report.all_required_succeeded() => {
            completed(RunTerminalCompletion::HardGateAbort)
        }
        RunTerminalSemanticOutcome::ScenarioComplete { .. }
        | RunTerminalSemanticOutcome::HardGateAbort { .. } => {
            quarantined(RunTerminalQuarantine::TerminalFailure)
        }
        RunTerminalSemanticOutcome::ExecutionError { .. } => {
            quarantined(RunTerminalQuarantine::ExecutionFailure)
        }
        RunTerminalSemanticOutcome::Recovery => quarantined(RunTerminalQuarantine::Recovery),
    }
}

const fn completed(completion: RunTerminalCompletion) -> RunTerminalDisposition {
    RunTerminalDisposition::Completed { completion }
}

const fn quarantined(quarantine: RunTerminalQuarantine) -> RunTerminalDisposition {
    RunTerminalDisposition::Quarantine { quarantine }
}

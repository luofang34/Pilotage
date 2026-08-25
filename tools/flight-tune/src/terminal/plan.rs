use serde::{Deserialize, Serialize};

use crate::{Digest, TuneError};

use super::digest::domain_digest;
use super::invalid_terminal;

/// The supported terminal plan schema.
pub const RUN_TERMINAL_PLAN_SCHEMA_VERSION: u16 = 1;

const PLAN_DOMAIN: &[u8] = b"pilotage.flight-tune.run-terminal-plan.v1\0";

/// One operation in the fixed terminal sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTerminalOperation {
    /// Stops the active simulator run.
    SimulatorStop,
    /// Stops the vehicle control path.
    ControlStop,
    /// Stops trace collection.
    TraceStop,
    /// Checks the supervised child state.
    ChildHealth,
    /// Joins the trace path.
    TraceShutdown,
    /// Terminates and reaps the child group.
    ChildTerminate,
}

/// The fixed terminal operation order.
pub const RUN_TERMINAL_OPERATION_ORDER: [RunTerminalOperation; 6] = [
    RunTerminalOperation::SimulatorStop,
    RunTerminalOperation::ControlStop,
    RunTerminalOperation::TraceStop,
    RunTerminalOperation::ChildHealth,
    RunTerminalOperation::TraceShutdown,
    RunTerminalOperation::ChildTerminate,
];

/// The external components that can exist for one prepared run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTerminalScope {
    /// The simulator and vehicle runtime started.
    Active,
    /// Only the vehicle runtime can require containment.
    RuntimeOnly,
    /// No external component started.
    NeverStarted,
}

/// The required state of one terminal operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "requirement", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTerminalRequirement {
    /// The operation must produce a terminal result.
    Required {
        /// The required operation.
        operation: RunTerminalOperation,
    },
    /// The operation must report that it was not required.
    NotRequired {
        /// The operation that did not apply.
        operation: RunTerminalOperation,
    },
}

impl RunTerminalRequirement {
    /// Returns the operation.
    #[must_use]
    pub const fn operation(self) -> RunTerminalOperation {
        match self {
            Self::Required { operation } | Self::NotRequired { operation } => operation,
        }
    }

    /// Reports whether the operation is required.
    #[must_use]
    pub const fn is_required(self) -> bool {
        matches!(self, Self::Required { .. })
    }
}

/// One immutable terminal plan made before external run mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTerminalPlan {
    schema_version: u16,
    scope: RunTerminalScope,
    requirements: Vec<RunTerminalRequirement>,
    plan_digest: Digest,
}

#[derive(Serialize)]
struct PlanDocument<'a> {
    schema_version: u16,
    scope: RunTerminalScope,
    requirements: &'a [RunTerminalRequirement],
}

impl RunTerminalPlan {
    /// Creates the fixed plan for one pre-run scope.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when canonical plan encoding fails.
    pub fn new(scope: RunTerminalScope) -> Result<Self, TuneError> {
        Self::from_requirements(scope, conservative_requirements(scope))
    }

    /// Creates a core-owned plan before any run component starts.
    ///
    /// Each flag uses [`RUN_TERMINAL_OPERATION_ORDER`]. An adapter can only
    /// consume the resulting immutable plan.
    pub(crate) fn from_requirements(
        scope: RunTerminalScope,
        requirements: [RunTerminalRequirement; 6],
    ) -> Result<Self, TuneError> {
        let mut plan = Self {
            schema_version: RUN_TERMINAL_PLAN_SCHEMA_VERSION,
            scope,
            requirements: requirements.into(),
            plan_digest: Digest::from_bytes([0; 32]),
        };
        plan.plan_digest = plan.recompute_digest()?;
        Ok(plan)
    }

    /// Validates the schema, scope profile, order, and canonical digest.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the plan is incomplete or changed.
    pub fn validate(&self) -> Result<(), TuneError> {
        self.validate_content()?;
        if self.plan_digest != self.recompute_digest()? || self.plan_digest.is_zero() {
            return Err(invalid_terminal("the terminal plan digest changed"));
        }
        Ok(())
    }

    /// Recomputes the domain-separated plan digest.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the plan is invalid or encoding fails.
    pub fn recompute_digest(&self) -> Result<Digest, TuneError> {
        self.validate_content()?;
        domain_digest(
            PLAN_DOMAIN,
            &PlanDocument {
                schema_version: self.schema_version,
                scope: self.scope,
                requirements: &self.requirements,
            },
            "run terminal plan",
        )
    }

    /// Returns the plan scope.
    #[must_use]
    pub const fn scope(&self) -> RunTerminalScope {
        self.scope
    }

    /// Returns the exact ordered operation requirements.
    #[must_use]
    pub fn requirements(&self) -> &[RunTerminalRequirement] {
        &self.requirements
    }

    /// Returns the canonical plan identity.
    #[must_use]
    pub const fn plan_digest(&self) -> Digest {
        self.plan_digest
    }

    fn validate_content(&self) -> Result<(), TuneError> {
        if self.schema_version != RUN_TERMINAL_PLAN_SCHEMA_VERSION
            || self.requirements.len() != RUN_TERMINAL_OPERATION_ORDER.len()
        {
            return Err(invalid_terminal("the terminal plan is incomplete"));
        }
        for (requirement, operation) in self
            .requirements
            .iter()
            .copied()
            .zip(RUN_TERMINAL_OPERATION_ORDER)
        {
            if requirement.operation() != operation {
                return Err(invalid_terminal(
                    "a terminal plan operation is missing, repeated, or out of order",
                ));
            }
        }
        validate_scope_requirements(self.scope, &self.requirements)
    }
}

const fn requirement(operation: RunTerminalOperation, required: bool) -> RunTerminalRequirement {
    if required {
        RunTerminalRequirement::Required { operation }
    } else {
        RunTerminalRequirement::NotRequired { operation }
    }
}

const fn conservative_requirements(scope: RunTerminalScope) -> [RunTerminalRequirement; 6] {
    match scope {
        RunTerminalScope::Active => required_operations(),
        RunTerminalScope::RuntimeOnly => [
            requirement(RunTerminalOperation::SimulatorStop, false),
            requirement(RunTerminalOperation::ControlStop, true),
            requirement(RunTerminalOperation::TraceStop, true),
            requirement(RunTerminalOperation::ChildHealth, true),
            requirement(RunTerminalOperation::TraceShutdown, true),
            requirement(RunTerminalOperation::ChildTerminate, true),
        ],
        RunTerminalScope::NeverStarted => not_required_operations(),
    }
}

const fn required_operations() -> [RunTerminalRequirement; 6] {
    [
        requirement(RunTerminalOperation::SimulatorStop, true),
        requirement(RunTerminalOperation::ControlStop, true),
        requirement(RunTerminalOperation::TraceStop, true),
        requirement(RunTerminalOperation::ChildHealth, true),
        requirement(RunTerminalOperation::TraceShutdown, true),
        requirement(RunTerminalOperation::ChildTerminate, true),
    ]
}

const fn not_required_operations() -> [RunTerminalRequirement; 6] {
    [
        requirement(RunTerminalOperation::SimulatorStop, false),
        requirement(RunTerminalOperation::ControlStop, false),
        requirement(RunTerminalOperation::TraceStop, false),
        requirement(RunTerminalOperation::ChildHealth, false),
        requirement(RunTerminalOperation::TraceShutdown, false),
        requirement(RunTerminalOperation::ChildTerminate, false),
    ]
}

fn validate_scope_requirements(
    scope: RunTerminalScope,
    requirements: &[RunTerminalRequirement],
) -> Result<(), TuneError> {
    let simulator_stop_required = requirements
        .first()
        .is_some_and(|requirement| requirement.is_required());
    let all_not_required = requirements
        .iter()
        .all(|requirement| !requirement.is_required());
    let valid = match scope {
        RunTerminalScope::Active => simulator_stop_required,
        RunTerminalScope::RuntimeOnly => !simulator_stop_required,
        RunTerminalScope::NeverStarted => all_not_required,
    };
    if !valid {
        return Err(invalid_terminal(
            "the terminal plan requirements do not match its pre-run scope",
        ));
    }
    Ok(())
}

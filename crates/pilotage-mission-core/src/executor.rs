//! The contract of a host executor (ADR-0041, ADR-0052): each operation a
//! bound action names is registered with its version, its typed parameters
//! and results, the capabilities it needs, how a cancel ends it, whether a
//! repeat with the same action identity is harmless, and whether the host
//! can tell its status after a restart. A host admits a document only when
//! every bound action names a registered operation of the right version
//! with parameters of the declared types, so an action the host cannot
//! carry out is refused before the run, not part of the way through it.

use std::collections::BTreeMap;

use crate::BoundAction;

/// The type of one parameter or result value, as a bound action writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// Text of at most this many bytes.
    Text {
        /// The most bytes the text may have.
        max_bytes: usize,
    },
    /// A whole number in a closed range.
    Integer {
        /// The least value.
        min: i64,
        /// The greatest value.
        max: i64,
    },
    /// `true` or `false`.
    Flag,
}

impl ValueKind {
    fn admits(self, value: &str) -> bool {
        match self {
            Self::Text { max_bytes } => value.len() <= max_bytes,
            Self::Integer { min, max } => {
                value.parse::<i64>().is_ok_and(|v| (min..=max).contains(&v))
            }
            Self::Flag => matches!(value, "true" | "false"),
        }
    }
}

/// One named value of an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueSpec {
    /// The value's name.
    pub name: &'static str,
    /// Its type.
    pub kind: ValueKind,
    /// A bound action must give it.
    pub required: bool,
}

/// How a cancel ends an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelSemantics {
    /// It stops at once and leaves no effect.
    Immediate,
    /// It stops at the next safe point; its partial effect is reported.
    AtSafePoint,
    /// It cannot be cancelled once begun; the cancel waits for its end.
    RunsToEnd,
}

/// One registered operation of a host executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationDescriptor {
    /// The executor that carries it out.
    pub executor: &'static str,
    /// The operation.
    pub operation: &'static str,
    /// The version of its parameters and results.
    pub version: u32,
    /// Its parameters.
    pub parameters: &'static [ValueSpec],
    /// Its results.
    pub results: &'static [ValueSpec],
    /// What the host must provide for it, such as `person`.
    pub capabilities: &'static [&'static str],
    /// How a cancel ends it.
    pub cancel: CancelSemantics,
    /// A repeat with the same action identity changes nothing.
    pub idempotent: bool,
    /// The host can tell after a restart whether it happened.
    pub status_query: bool,
}

/// Why a host does not admit a bound action.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BoundRefused {
    /// No registered operation has that executor and name.
    #[error("the host has no operation {executor}/{operation}")]
    Unregistered {
        /// The action's executor.
        executor: String,
        /// The action's operation.
        operation: String,
    },
    /// The operation is registered in another version.
    #[error("{executor}/{operation} version {asked} is not version {registered}")]
    Version {
        /// The action's executor.
        executor: String,
        /// The action's operation.
        operation: String,
        /// The version the action names.
        asked: u32,
        /// The version the host registers.
        registered: u32,
    },
    /// A required parameter is missing.
    #[error("{operation} needs the parameter {name}")]
    Missing {
        /// The action's operation.
        operation: String,
        /// The missing parameter.
        name: &'static str,
    },
    /// A parameter the operation does not declare.
    #[error("{operation} has no parameter {name}")]
    Unknown {
        /// The action's operation.
        operation: String,
        /// The parameter.
        name: String,
    },
    /// A parameter value is not of its declared type.
    #[error("{operation} parameter {name} is not {kind:?}: {value:?}")]
    Type {
        /// The action's operation.
        operation: String,
        /// The parameter.
        name: &'static str,
        /// Its declared type.
        kind: ValueKind,
        /// The value given.
        value: String,
    },
}

/// The operations a host can carry out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutorRegistry {
    operations: &'static [OperationDescriptor],
}

impl ExecutorRegistry {
    /// A host with no executor: it admits no bound action.
    pub const NONE: Self = Self { operations: &[] };

    /// A registry of `operations`.
    #[must_use]
    pub const fn new(operations: &'static [OperationDescriptor]) -> Self {
        Self { operations }
    }

    /// The registered operations.
    #[must_use]
    pub fn operations(&self) -> &'static [OperationDescriptor] {
        self.operations
    }

    /// The descriptor `action` names, when its version and its parameters
    /// match it.
    ///
    /// # Errors
    ///
    /// [`BoundRefused`] for an operation the host does not register, another
    /// version, or parameters missing, unknown or of the wrong type.
    pub fn admit(
        &self,
        action: &BoundAction,
    ) -> Result<&'static OperationDescriptor, BoundRefused> {
        let descriptor = self
            .operations
            .iter()
            .find(|d| d.executor == action.executor && d.operation == action.operation)
            .ok_or_else(|| BoundRefused::Unregistered {
                executor: action.executor.clone(),
                operation: action.operation.clone(),
            })?;
        if action.version != descriptor.version {
            return Err(BoundRefused::Version {
                executor: action.executor.clone(),
                operation: action.operation.clone(),
                asked: action.version,
                registered: descriptor.version,
            });
        }
        admit_values(&action.operation, descriptor.parameters, &action.parameters)?;
        Ok(descriptor)
    }
}

/// Whether `values`, by name, are what `specs` declare for `operation`:
/// none unknown, every required one given, each of its declared type.
///
/// # Errors
///
/// [`BoundRefused`] naming the first value that is not.
pub fn admit_values(
    operation: &str,
    specs: &[ValueSpec],
    values: &BTreeMap<String, String>,
) -> Result<(), BoundRefused> {
    if let Some(name) = values
        .keys()
        .find(|n| !specs.iter().any(|s| s.name == n.as_str()))
    {
        return Err(BoundRefused::Unknown {
            operation: operation.to_owned(),
            name: name.clone(),
        });
    }
    for spec in specs {
        match values.get(spec.name) {
            None if spec.required => {
                return Err(BoundRefused::Missing {
                    operation: operation.to_owned(),
                    name: spec.name,
                });
            }
            Some(value) if !spec.kind.admits(value) => {
                return Err(BoundRefused::Type {
                    operation: operation.to_owned(),
                    name: spec.name,
                    kind: spec.kind,
                    value: value.clone(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

/// A person confirms, or declines, what the operation asks.
pub const WAIT_FOR_PERSON: OperationDescriptor = OperationDescriptor {
    executor: "person",
    operation: "confirm",
    version: 1,
    parameters: &[
        ValueSpec {
            name: "question",
            kind: ValueKind::Text { max_bytes: 512 },
            required: true,
        },
        ValueSpec {
            name: "subject",
            kind: ValueKind::Text { max_bytes: 1024 },
            required: false,
        },
        ValueSpec {
            name: "timeout_s",
            kind: ValueKind::Integer {
                min: 1,
                max: 86_400,
            },
            required: false,
        },
    ],
    results: &[
        ValueSpec {
            name: "confirmed",
            kind: ValueKind::Flag,
            required: true,
        },
        ValueSpec {
            name: "by",
            kind: ValueKind::Text { max_bytes: 128 },
            required: true,
        },
    ],
    capabilities: &["person"],
    cancel: CancelSemantics::Immediate,
    idempotent: true,
    status_query: true,
};

/// The host reads one named value, such as a sensor reading.
pub const READ_VALUE: OperationDescriptor = OperationDescriptor {
    executor: "host",
    operation: "read-value",
    version: 1,
    parameters: &[ValueSpec {
        name: "source",
        kind: ValueKind::Text { max_bytes: 128 },
        required: true,
    }],
    results: &[ValueSpec {
        name: "value",
        kind: ValueKind::Text { max_bytes: 1024 },
        required: true,
    }],
    capabilities: &["data-source"],
    cancel: CancelSemantics::Immediate,
    idempotent: true,
    status_query: false,
};

/// The host writes a report of a value. The report of one operation is
/// written once: a repeat finds it, and the host can tell after a restart
/// whether it exists. A cancel waits for the write to end.
pub const WRITE_REPORT: OperationDescriptor = OperationDescriptor {
    executor: "host",
    operation: "write-report",
    version: 1,
    parameters: &[
        ValueSpec {
            name: "title",
            kind: ValueKind::Text { max_bytes: 512 },
            required: true,
        },
        ValueSpec {
            name: "value",
            kind: ValueKind::Text { max_bytes: 1024 },
            required: true,
        },
    ],
    results: &[ValueSpec {
        name: "report",
        kind: ValueKind::Text { max_bytes: 256 },
        required: true,
    }],
    capabilities: &["report-store"],
    cancel: CancelSemantics::RunsToEnd,
    idempotent: true,
    status_query: true,
};

/// A rule holds a value to a limit: the value is within the limit when the
/// number it starts with is at most the limit. The rule reads and changes
/// nothing, so a repeat is harmless and any host carries it out.
pub const AT_MOST: OperationDescriptor = OperationDescriptor {
    executor: "rule",
    operation: "at-most",
    version: 1,
    parameters: &[
        ValueSpec {
            name: "limit",
            kind: ValueKind::Text { max_bytes: 64 },
            required: true,
        },
        ValueSpec {
            name: "value",
            kind: ValueKind::Text { max_bytes: 1024 },
            required: true,
        },
    ],
    results: &[
        ValueSpec {
            name: "verdict",
            kind: ValueKind::Text { max_bytes: 1024 },
            required: true,
        },
        ValueSpec {
            name: "within",
            kind: ValueKind::Flag,
            required: true,
        },
    ],
    capabilities: &[],
    cancel: CancelSemantics::Immediate,
    idempotent: true,
    status_query: false,
};

pub mod rule;

#[cfg(test)]
mod tests;

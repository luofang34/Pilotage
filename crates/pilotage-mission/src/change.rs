//! What a revision changes, as a set of classes, and whether an envelope
//! permits all of them (ADR-0052). The set comes from comparing the old and
//! the new content, never from the kind of request that asked for the
//! change. A task kind says what its fields mean; this module only collects
//! the classes and checks each one against its permission.

use std::fmt;

use pilotage_protocol::wire;

/// One class of change, each under its own permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeClass {
    /// How the work is done: its tasks, their order, speed or height.
    Plan,
    /// Who holds the work.
    Assignment,
    /// What the work must achieve.
    Objective,
}

impl ChangeClass {
    /// The envelope permission the class needs.
    #[must_use]
    pub fn permission(self) -> &'static str {
        match self {
            Self::Plan => "permit_revise",
            Self::Assignment => "permit_allocate",
            Self::Objective => "permit_change_objective",
        }
    }

    fn permitted(self, envelope: &wire::AdaptationEnvelope) -> bool {
        match self {
            Self::Plan => envelope.permit_revise,
            Self::Assignment => envelope.permit_allocate,
            Self::Objective => envelope.permit_change_objective,
        }
    }
}

/// One change, with the difference that puts it in its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The class of the change.
    pub class: ChangeClass,
    /// What differs, in words, such as "the area".
    pub what: String,
}

/// Every change a revision makes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    changes: Vec<Change>,
}

impl ChangeSet {
    /// Adds a change of `class`.
    pub fn add(&mut self, class: ChangeClass, what: impl Into<String>) {
        self.changes.push(Change {
            class,
            what: what.into(),
        });
    }

    /// Adds every change of `other`.
    pub fn extend(&mut self, other: Self) {
        self.changes.extend(other.changes);
    }

    /// The changes, in the order they were found.
    #[must_use]
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// Whether the set has a change of `class`.
    #[must_use]
    pub fn has(&self, class: ChangeClass) -> bool {
        self.changes.iter().any(|c| c.class == class)
    }
}

/// Why an envelope does not permit a change set: each class it lacks, with
/// the changes of that class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unpermitted {
    /// The classes without permission, each with what differs.
    pub missing: Vec<(ChangeClass, Vec<String>)>,
}

impl fmt::Display for Unpermitted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self
            .missing
            .iter()
            .map(|(class, what)| format!("{} for {}", class.permission(), what.join(", ")))
            .collect();
        write!(f, "the envelope lacks {}", parts.join("; "))
    }
}

/// Whether `envelope` permits every class in `changes`.
///
/// # Errors
///
/// [`Unpermitted`] naming each class without its permission.
pub fn authorize(
    envelope: &wire::AdaptationEnvelope,
    changes: &ChangeSet,
) -> Result<(), Unpermitted> {
    let mut missing: Vec<(ChangeClass, Vec<String>)> = Vec::new();
    for change in &changes.changes {
        if change.class.permitted(envelope) {
            continue;
        }
        match missing.iter_mut().find(|(class, _)| *class == change.class) {
            Some((_, what)) => what.push(change.what.clone()),
            None => missing.push((change.class, vec![change.what.clone()])),
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        missing.sort_by_key(|(class, _)| *class);
        Err(Unpermitted { missing })
    }
}

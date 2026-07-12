//! The alert policy: per-class escalation deadlines and inhibit wiring.
//!
//! Aircraft-specific inhibit schedules never live in the manager; they live
//! in a validated [`AlertProfile`], constructed like the display profiles
//! in `pilotage-instrument-state` — through a constructor that rejects an
//! ill-formed policy up front. Validation refuses a zero escalation
//! deadline, more inhibit rules than the fixed budget, a rule that would
//! inhibit a warning, and a rule naming an identity this build does not
//! know. [`AlertProfile::simulator`] is benchmark data only and implies no
//! aircraft approval.

use crate::class::AlertClass;
use crate::condition::{AlertCondition, AlertId, DynFault, SystemNote, class_of};
use crate::event::FlightPhase;

/// Most inhibit rules a profile may carry.
pub const MAX_INHIBIT_RULES: usize = 16;

/// One inhibit rule: `id` is suppressed while the aircraft is in `phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InhibitRule {
    /// The identity to inhibit. Must be a known, non-warning identity.
    pub id: AlertId,
    /// The phase the rule applies in.
    pub phase: FlightPhase,
}

/// Why an [`AlertProfile`] could not be constructed.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    /// An escalation deadline is zero, which would re-trigger the aural
    /// every step.
    #[error("escalation deadline for {class:?} must be positive")]
    NonPositiveEscalation {
        /// The class whose deadline was zero.
        class: AlertClass,
    },
    /// More inhibit rules were supplied than the fixed budget holds.
    #[error("profile carries {count} inhibit rules, over the fixed budget")]
    TooManyInhibits {
        /// The number of rules supplied.
        count: usize,
    },
    /// A rule targets a warning-class identity; warnings may not be
    /// inhibited.
    #[error("inhibit rule targets warning alert {id:?}, which may not be inhibited")]
    UninhibitableAlert {
        /// The offending identity.
        id: AlertId,
    },
    /// A rule targets an identity this build does not know.
    #[error("inhibit rule targets unknown alert {id:?}")]
    UnknownInhibitAlert {
        /// The unknown identity.
        id: AlertId,
    },
}

/// Escalation deadlines and inhibit wiring for the alert manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertProfile {
    warning_escalation_ms: u64,
    caution_escalation_ms: u64,
    advisory_escalation_ms: u64,
    inhibits: [Option<InhibitRule>; MAX_INHIBIT_RULES],
}

impl AlertProfile {
    /// Builds a profile after validating deadlines and inhibit wiring.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] when an escalation deadline is zero, when
    /// there are more inhibit rules than the fixed budget, when a rule
    /// targets a warning, or when a rule names an unknown identity.
    pub fn new(
        warning_escalation_ms: u64,
        caution_escalation_ms: u64,
        advisory_escalation_ms: u64,
        inhibits: &[InhibitRule],
    ) -> Result<Self, ProfileError> {
        if warning_escalation_ms == 0 {
            return Err(ProfileError::NonPositiveEscalation {
                class: AlertClass::Warning,
            });
        }
        if caution_escalation_ms == 0 {
            return Err(ProfileError::NonPositiveEscalation {
                class: AlertClass::Caution,
            });
        }
        if advisory_escalation_ms == 0 {
            return Err(ProfileError::NonPositiveEscalation {
                class: AlertClass::Advisory,
            });
        }
        if inhibits.len() > MAX_INHIBIT_RULES {
            return Err(ProfileError::TooManyInhibits {
                count: inhibits.len(),
            });
        }
        let mut wiring = [None; MAX_INHIBIT_RULES];
        let mut i = 0;
        while i < inhibits.len() {
            let rule = inhibits[i];
            match class_of(rule.id) {
                None => return Err(ProfileError::UnknownInhibitAlert { id: rule.id }),
                Some(AlertClass::Warning) => {
                    return Err(ProfileError::UninhibitableAlert { id: rule.id });
                }
                Some(_) => {}
            }
            wiring[i] = Some(rule);
            i += 1;
        }
        Ok(Self {
            warning_escalation_ms,
            caution_escalation_ms,
            advisory_escalation_ms,
            inhibits: wiring,
        })
    }

    /// A benchmark-data-only profile. Its deadlines (warning 5 s, caution
    /// 10 s, advisory 20 s) and its two inhibit rules — turn-rate advisory
    /// during takeoff, database-stale status on approach — are simulator
    /// inputs, not an approval for any aircraft.
    pub fn simulator() -> Self {
        let mut inhibits = [None; MAX_INHIBIT_RULES];
        inhibits[0] = Some(InhibitRule {
            id: AlertCondition::TurnSlip(DynFault::TurnRateInvalid).id(),
            phase: FlightPhase::Takeoff,
        });
        inhibits[1] = Some(InhibitRule {
            id: AlertCondition::System(SystemNote::DatabaseStale).id(),
            phase: FlightPhase::Approach,
        });
        Self {
            warning_escalation_ms: 5_000,
            caution_escalation_ms: 10_000,
            advisory_escalation_ms: 20_000,
            inhibits,
        }
    }

    /// The escalation deadline for a class. Silent classes never escalate.
    pub fn escalation_ms(&self, class: AlertClass) -> u64 {
        match class {
            AlertClass::Warning => self.warning_escalation_ms,
            AlertClass::Caution => self.caution_escalation_ms,
            AlertClass::Advisory => self.advisory_escalation_ms,
            AlertClass::Status | AlertClass::Maintenance => u64::MAX,
        }
    }

    /// Whether `id` is inhibited in `phase` by any rule.
    pub fn is_inhibited(&self, id: AlertId, phase: FlightPhase) -> bool {
        self.inhibits
            .iter()
            .flatten()
            .any(|rule| rule.id == id && rule.phase == phase)
    }
}

#[cfg(test)]
mod tests;

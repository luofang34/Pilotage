//! The environmental conditions one scored window was flown under.
//!
//! The simulator owns applying a condition set; the vehicle port only
//! observes the result. What the vehicle port owns is the guarantee that a
//! scored window was flown under one condition set: a value that changes
//! part-way through a stimulus would make the measured response the
//! response to two different worlds.

use std::collections::BTreeMap;

use flight_tune::ScenarioFrame;

use super::AviateRuntimeError;
use super::math::require_finite;

/// The condition values one run has observed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConditionLedger {
    observed: BTreeMap<String, f64>,
    locked: bool,
}

impl ConditionLedger {
    /// Creates one empty ledger.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            observed: BTreeMap::new(),
            locked: false,
        }
    }

    /// Clears the ledger for a new run.
    pub fn clear(&mut self) {
        self.observed.clear();
        self.locked = false;
    }

    /// Freezes the observed conditions for the length of a scored window.
    ///
    /// A trial that applies no conditions freezes the empty set. The
    /// invariant is that the set does not change inside the window, and an
    /// empty set satisfies it until a condition appears, which the locked
    /// path then refuses.
    pub const fn lock(&mut self) {
        self.locked = true;
    }

    /// Releases the freeze at the end of a scored window.
    pub const fn unlock(&mut self) {
        self.locked = false;
    }

    /// Records the conditions one frame reports.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a value is not finite, or when a
    /// value changes while a scored window is frozen.
    pub fn observe(&mut self, frame: &ScenarioFrame) -> Result<(), AviateRuntimeError> {
        for (name, value) in &frame.applied_conditions {
            let value = require_finite("applied condition", *value)?;
            match self.observed.get(name) {
                Some(previous) if previous.to_bits() == value.to_bits() => {}
                Some(_) | None if !self.locked => {
                    self.observed.insert(name.clone(), value);
                }
                Some(_) => {
                    return Err(AviateRuntimeError::ConditionChanged { name: name.clone() });
                }
                None => {
                    return Err(AviateRuntimeError::ConditionChanged { name: name.clone() });
                }
            }
        }
        if self.locked && frame.applied_conditions.len() != self.observed.len() {
            return Err(AviateRuntimeError::ConditionChanged {
                name: "the applied condition set".to_owned(),
            });
        }
        Ok(())
    }

    /// The conditions this run has observed.
    #[must_use]
    pub const fn observed(&self) -> &BTreeMap<String, f64> {
        &self.observed
    }

    /// Whether a scored window has frozen the conditions.
    #[must_use]
    pub const fn is_locked(&self) -> bool {
        self.locked
    }
}

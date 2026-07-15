//! The airframe source-selection policy: thresholds, persistence,
//! hysteresis, priority, manual selection, reversion, and return-to-primary,
//! all in one validated place.
//!
//! Every knob the comparator reads lives here, constructed only through
//! [`AirframeSourcePolicy::new`] or [`AirframeSourcePolicy::simulator`], so an
//! inverted agree/miscompare band (which would let the miscompare state
//! chatter) or an empty priority order cannot exist. The simulator profile's
//! numbers are benchmark data for the simulator display only; they imply no
//! aircraft approval.

use pilotage_alerts::MiscompareFault;

use crate::source_compare::{SourceId, SourceList};

const DEG: f32 = core::f32::consts::PI / 180.0;

/// Why an [`AirframeSourcePolicy`] could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePolicyError {
    /// The priority order is empty, so no source could ever be selected.
    EmptyPriority,
    /// An agreement threshold is NaN or infinite.
    NonFiniteThreshold,
    /// The miscompare threshold does not sit strictly beyond the agreement
    /// threshold, so the comparison state could chatter.
    NoHysteresis,
}

/// The raw policy inputs [`AirframeSourcePolicy::new`] validates. Grouped so
/// construction takes one datum, not a long argument list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourcePolicyLimits {
    /// Selection priority: index 0 is the primary, the rest are the ordered
    /// fallbacks a reversion walks.
    pub priority: SourceList,
    /// Difference at or below which comparable samples agree (the hysteresis
    /// exit), in the metric's canonical unit (radians, meters,
    /// meters/second).
    pub agree_within: f32,
    /// Difference at or beyond which comparable samples disagree (the
    /// hysteresis entry); must sit strictly beyond `agree_within`.
    pub miscompare_beyond: f32,
    /// Largest source-time spread for a pair to count as simultaneous, ms.
    pub skew_budget_ms: u64,
    /// Largest receive age for a sample to be a usable simultaneous sample,
    /// ms; older samples are neither compared nor selectable.
    pub max_age_ms: u64,
    /// How long a disagreement must persist before it is a sustained
    /// miscompare, ms; transient spikes below this never annunciate.
    pub sustain_ms: u64,
    /// How long the primary must be continuously available before selection
    /// returns to it after a reversion, ms; the return-chatter guard.
    pub return_stable_ms: u64,
    /// Whether a failed primary may revert to a lower-priority source.
    pub allow_reversion: bool,
    /// Whether a manual pilot selection is honored.
    pub allow_manual: bool,
    /// Whether a strictly higher-integrity source may be selected out of a
    /// sustained two-source disagreement.
    pub use_integrity_tiebreak: bool,
    /// Whether declared accuracies widen the agreement band.
    pub use_accuracy_band: bool,
}

/// A validated source-selection policy for one display function.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirframeSourcePolicy {
    limits: SourcePolicyLimits,
}

impl AirframeSourcePolicy {
    /// Builds a policy after validating its thresholds and priority.
    ///
    /// # Errors
    ///
    /// Returns [`SourcePolicyError`] when the priority order is empty, an
    /// agreement threshold is non-finite, or the miscompare threshold does
    /// not sit strictly beyond the agreement threshold.
    pub fn new(limits: SourcePolicyLimits) -> Result<Self, SourcePolicyError> {
        if limits.priority.is_empty() {
            return Err(SourcePolicyError::EmptyPriority);
        }
        if !(limits.agree_within.is_finite() && limits.miscompare_beyond.is_finite()) {
            return Err(SourcePolicyError::NonFiniteThreshold);
        }
        if !(limits.agree_within >= 0.0 && limits.miscompare_beyond > limits.agree_within) {
            return Err(SourcePolicyError::NoHysteresis);
        }
        Ok(Self { limits })
    }

    /// The simulator benchmark profile for one display function. Thresholds
    /// are benchmark inputs in the metric's canonical unit (roughly: attitude
    /// 2°/5°, heading 3°/6°, altitude 30 m/60 m, airspeed 2.5/5 m/s — applied
    /// in m/s even to sources that report knots) with a two-source priority,
    /// reversion and manual selection enabled, and integrity tiebreak on.
    /// They are profile data only and imply no aircraft approval.
    #[must_use]
    pub fn simulator(function: MiscompareFault) -> Self {
        let (agree_within, miscompare_beyond) = match function {
            MiscompareFault::Attitude => (2.0 * DEG, 5.0 * DEG),
            MiscompareFault::Heading => (3.0 * DEG, 6.0 * DEG),
            MiscompareFault::Altitude => (30.0, 60.0),
            MiscompareFault::Airspeed => (2.5, 5.0),
        };
        let mut priority = SourceList::new();
        let _ = priority.try_push(SourceId(1));
        let _ = priority.try_push(SourceId(2));
        Self {
            limits: SourcePolicyLimits {
                priority,
                agree_within,
                miscompare_beyond,
                skew_budget_ms: 50,
                max_age_ms: 500,
                sustain_ms: 1_000,
                return_stable_ms: 3_000,
                allow_reversion: true,
                allow_manual: true,
                use_integrity_tiebreak: true,
                use_accuracy_band: false,
            },
        }
    }

    /// The selection priority order; index 0 is the primary.
    #[must_use]
    pub fn priority(&self) -> &SourceList {
        &self.limits.priority
    }

    /// The configured primary source, if the priority order is non-empty.
    #[must_use]
    pub fn primary(&self) -> Option<SourceId> {
        self.limits.priority.first()
    }

    /// The hysteresis exit (agreement) threshold.
    #[must_use]
    pub fn agree_within(&self) -> f32 {
        self.limits.agree_within
    }

    /// The hysteresis entry (miscompare) threshold.
    #[must_use]
    pub fn miscompare_beyond(&self) -> f32 {
        self.limits.miscompare_beyond
    }

    /// The pairwise source-time skew budget, ms.
    #[must_use]
    pub fn skew_budget_ms(&self) -> u64 {
        self.limits.skew_budget_ms
    }

    /// The receive-age limit for a usable sample, ms.
    #[must_use]
    pub fn max_age_ms(&self) -> u64 {
        self.limits.max_age_ms
    }

    /// The sustained-miscompare persistence threshold, ms.
    #[must_use]
    pub fn sustain_ms(&self) -> u64 {
        self.limits.sustain_ms
    }

    /// The return-to-primary stability threshold, ms.
    #[must_use]
    pub fn return_stable_ms(&self) -> u64 {
        self.limits.return_stable_ms
    }

    /// Whether reversion off a failed primary is permitted.
    #[must_use]
    pub fn allow_reversion(&self) -> bool {
        self.limits.allow_reversion
    }

    /// Whether a manual pilot selection is honored.
    #[must_use]
    pub fn allow_manual(&self) -> bool {
        self.limits.allow_manual
    }

    /// Whether a strictly higher-integrity source may break a two-source
    /// disagreement.
    #[must_use]
    pub fn use_integrity_tiebreak(&self) -> bool {
        self.limits.use_integrity_tiebreak
    }

    /// Whether declared accuracies widen the agreement band.
    #[must_use]
    pub fn use_accuracy_band(&self) -> bool {
        self.limits.use_accuracy_band
    }
}

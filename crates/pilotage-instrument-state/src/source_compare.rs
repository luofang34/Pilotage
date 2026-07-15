//! Per-display-function source comparison, miscompare, and deterministic
//! reversion (SRC-01).
//!
//! A display function (attitude, heading, altitude, airspeed) may be fed by
//! several independent candidate sources. This module compares only the
//! samples that are genuinely simultaneous and on the same datum, decides
//! which source a panel displays, and annunciates disagreement — without
//! ever averaging sources, voting, or picking the closest-looking value.
//!
//! Comparison is honest about what it cannot establish. Two sources that
//! disagree are ambiguous: the comparator keeps the priority-selected source
//! and raises a miscompare rather than guessing which is correct, unless
//! independent integrity evidence justifies a selection. Stale, reordered,
//! old-epoch, skewed, or datum-incompatible data are never treated as
//! simultaneous valid samples. Selection follows a validated
//! [`AirframeSourcePolicy`]; reversion away from a failed primary and the
//! return to it are governed so neither can chatter.
//!
//! Software comparison alone is not evidence of hardware independence: this
//! module claims no physical independence, no common-cause mitigation, and
//! no certification credit. It is pure, bounded, allocation-free, and reads
//! no interior clock — time enters only as caller-supplied milliseconds.

use pilotage_alerts::{AlertEvent, MiscompareFault};

mod comparator;
mod measure;
mod policy;

pub use comparator::SourceComparator;
pub use measure::{
    AttitudeMeasure, FrameTag, HeadingMeasure, ScalarMeasure, ScalarUnit, SourceAltitude,
    VectorMeasure,
};
pub use policy::{AirframeSourcePolicy, SourcePolicyError, SourcePolicyLimits};

/// Largest number of candidate sources one display function accepts. The
/// bound keeps every buffer and the pairwise comparison allocation-free.
pub const MAX_SOURCES: usize = 8;

/// Identity of one candidate source. Distinct physical/logical sensors carry
/// distinct ids; the id is what a display annunciates as its selected source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u8);

/// A source's clock/epoch tag. A source restart or clock reset advances its
/// epoch; samples on different epochs are on different clocks and are never
/// compared as simultaneous. Wraps rather than overflowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceEpoch(pub u32);

/// Independent integrity evidence a source declares about its own sample.
///
/// Ordered least-to-most trustworthy so a strictly higher level can justify
/// selecting one disagreeing source over another. It is evidence *about* a
/// source, never a substitute for comparison, and never fuses values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum IntegrityLevel {
    /// No integrity evidence; cannot justify selection over a peer.
    #[default]
    None,
    /// Reduced integrity evidence.
    Low,
    /// Full integrity evidence.
    High,
}

/// A per-function measurement that can be checked for datum compatibility and
/// compared for agreement in its own natural metric.
///
/// Implementors define what "same datum/units" means and how far apart two
/// samples are. The comparator calls [`Self::difference`] only for a pair
/// that is both [`Self::well_formed`] and [`Self::datum_compatible`], so an
/// implementation never has to defend against ill-formed or cross-datum
/// inputs inside `difference`.
pub trait Comparable: Copy {
    /// Whether this sample is finite and carries a fully declared datum, so
    /// it can participate in a comparison at all. An ill-formed sample is
    /// unusable, never silently coerced.
    fn well_formed(&self) -> bool;

    /// Whether `self` and `other` share compatible units/datum. Incompatible
    /// datums make a pair [`ComparisonState::NotComparable`] — never
    /// `Agree`, never `Miscompare`, and never converted implicitly.
    fn datum_compatible(&self, other: &Self) -> bool;

    /// The magnitude of disagreement between two compatible samples, in the
    /// metric's canonical unit (radians, meters, meters/second) regardless of
    /// the unit the samples were reported in. Always non-negative.
    fn difference(&self, other: &Self) -> f32;
}

/// One candidate source's sample for a display function, with the provenance
/// the comparator needs to decide coherence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate<M> {
    /// Which source produced the sample.
    pub source: SourceId,
    /// The source's clock/epoch; samples on different epochs never compare.
    pub epoch: SourceEpoch,
    /// When the source acquired the sample, milliseconds on the shared
    /// monotonic scale. Drives the pairwise skew budget.
    pub source_time_ms: u64,
    /// When the sample was received, milliseconds on the shared scale.
    /// Drives staleness relative to the step's `now`.
    pub receive_time_ms: u64,
    /// Per-source wrapping sequence counter. A sample whose sequence does not
    /// advance in wrapping (serial-number) order is a replay or reorder and
    /// is dropped; `u32::MAX → 0` is a normal single advance.
    pub sequence: u32,
    /// The source declares the sample valid. A `false` sample is unusable.
    pub valid: bool,
    /// Independent integrity evidence about this sample.
    pub integrity: IntegrityLevel,
    /// Declared accuracy (1-sigma) in the metric's canonical unit (radians,
    /// meters, meters/second); widens the agreement band under policy but
    /// never selects a source.
    pub accuracy: f32,
    /// The measured value and its datum.
    pub measurement: M,
}

/// The four comparison outcomes for one display function this step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComparisonState {
    /// Fewer than two usable, coherent samples exist, so nothing can be
    /// compared. Fail-closed default: comparison is established, never
    /// assumed.
    #[default]
    InsufficientSources,
    /// Two or more usable samples exist, but no pair shares a compatible
    /// datum within the skew budget, so no meaningful comparison exists.
    NotComparable,
    /// Comparable samples agree within tolerance.
    Agree,
    /// Comparable samples disagree beyond tolerance and the disagreement has
    /// persisted past the sustain threshold. Transient spikes never reach
    /// this state.
    Miscompare,
}

/// The comparator's decision for one display function this step.
///
/// A display renders the value of [`Self::selected`] and always annunciates
/// which source that is; `None` means no usable source and no value is shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceComparison {
    /// The source a panel must display, or `None` when every source failed.
    pub selected: Option<SourceId>,
    /// The four-state comparison result.
    pub state: ComparisonState,
    /// The displayed source is not the configured primary (reverted after a
    /// primary failure, chosen by integrity evidence, or manually selected),
    /// so the display annunciates the non-primary selection.
    pub reverted: bool,
    /// The selection is the pilot's manual choice rather than an automatic one.
    pub manual: bool,
    /// The sustained-miscompare fault level for this function, present while
    /// [`Self::state`] is [`ComparisonState::Miscompare`].
    pub fault: Option<MiscompareFault>,
    /// The typed ALR-01 transition on the rising/falling edge of the
    /// sustained miscompare, ready to forward to the alert manager. `None`
    /// on steps with no edge.
    pub transition: Option<AlertEvent>,
    /// Output identity, advanced (wrapping) whenever the selected source,
    /// reversion, or comparison state changes.
    pub generation: u32,
}

/// A bounded, duplicate-free set of source ids, allocation-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceList {
    ids: [SourceId; MAX_SOURCES],
    len: usize,
}

impl Default for SourceList {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceList {
    /// An empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ids: [SourceId(0); MAX_SOURCES],
            len: 0,
        }
    }

    /// Appends `id`, returning `false` if the list is full or already holds
    /// it (duplicates are rejected, never silently merged).
    pub fn try_push(&mut self, id: SourceId) -> bool {
        if self.len >= MAX_SOURCES || self.contains(id) {
            return false;
        }
        self.ids[self.len] = id;
        self.len += 1;
        true
    }

    /// Whether `id` is present.
    #[must_use]
    pub fn contains(&self, id: SourceId) -> bool {
        self.as_slice().contains(&id)
    }

    /// The ids in insertion order.
    #[must_use]
    pub fn as_slice(&self) -> &[SourceId] {
        &self.ids[..self.len]
    }

    /// The first id, if any.
    #[must_use]
    pub fn first(&self) -> Option<SourceId> {
        self.as_slice().first().copied()
    }

    /// Number of ids held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests;

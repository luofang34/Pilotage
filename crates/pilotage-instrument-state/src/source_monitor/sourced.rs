//! The inseparable value-and-source result.
//!
//! A displayed value can only be obtained together with the source it came
//! from: [`Sourced`] holds both, is built only from a single selected
//! candidate, and exposes no constructor that pairs a value with a foreign
//! source. A panel therefore cannot render one source's value under another
//! source's label, and on reversion the value and the label switch together
//! because they are one value.

use pilotage_frames::Quat;

use crate::source_compare::{
    AttitudeMeasure, Candidate, Comparable, ComparisonState, HeadingMeasure, ScalarMeasure,
    SourceAltitude, SourceComparison, SourceId,
};

/// The display value a measurement contributes, read from the same candidate
/// as the source id so the two are never separated.
pub(crate) trait DisplayValue: Comparable {
    /// The value type a panel renders for this function.
    type Value: Copy;
    /// The display value carried by this sample.
    fn display_value(&self) -> Self::Value;
}

impl DisplayValue for AttitudeMeasure {
    type Value = Quat;
    fn display_value(&self) -> Quat {
        self.quat
    }
}

impl DisplayValue for HeadingMeasure {
    type Value = f32;
    fn display_value(&self) -> f32 {
        self.heading_rad
    }
}

impl DisplayValue for SourceAltitude {
    type Value = f32;
    fn display_value(&self) -> f32 {
        self.value_m
    }
}

impl DisplayValue for ScalarMeasure {
    type Value = f32;
    fn display_value(&self) -> f32 {
        self.value
    }
}

/// A display value inseparable from the source it was taken from.
///
/// Both parts are read from one selected candidate at construction; there is
/// no public constructor and no way to re-pair the value with a different
/// source, so the identity is load-bearing, not decorative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sourced<T> {
    value: T,
    source: SourceId,
}

impl<T: Copy> Sourced<T> {
    /// The selected source's own display value.
    #[must_use]
    pub fn value(&self) -> T {
        self.value
    }

    /// The source the value came from — always the one that produced it.
    #[must_use]
    pub fn source(&self) -> SourceId {
        self.source
    }

    pub(crate) fn from_candidate<M>(candidate: &Candidate<M>) -> Self
    where
        M: DisplayValue<Value = T>,
    {
        Self {
            value: candidate.measurement.display_value(),
            source: candidate.source,
        }
    }
}

/// One display function's selected value bound to its source, with the
/// reversion and comparison state a panel annunciates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourcedFunction<T> {
    /// The selected source's value bound to its id; `None` when every
    /// candidate failed.
    pub selected: Option<Sourced<T>>,
    /// The selection is not the configured primary (reverted after a failure,
    /// chosen by integrity evidence, or manual), so a panel annunciates a
    /// non-primary source.
    pub reverted: bool,
    /// The four-state comparison result driving the miscompare cue.
    pub state: ComparisonState,
}

// Manual, so `SourcedFunction<Quat>` does not require `Quat: Default`.
impl<T> Default for SourcedFunction<T> {
    fn default() -> Self {
        Self {
            selected: None,
            reverted: false,
            state: ComparisonState::InsufficientSources,
        }
    }
}

/// The candidate the comparator selected this step (the first with the
/// selected id), or `None` when nothing was selected. The one lookup that
/// both the displayed value and its label are drawn from, so they can never
/// name different sources.
pub(crate) fn selected_candidate<'a, M>(
    candidates: &'a [Candidate<M>],
    comparison: &SourceComparison,
) -> Option<&'a Candidate<M>> {
    comparison
        .selected
        .and_then(|id| candidates.iter().find(|c| c.source == id))
}

/// Binds the comparator's selected source to that source's own candidate
/// value. The value comes from the candidate the id names, so value and id
/// can never diverge.
pub(crate) fn sourced_function<M: DisplayValue>(
    candidates: &[Candidate<M>],
    comparison: &SourceComparison,
) -> SourcedFunction<M::Value> {
    SourcedFunction {
        selected: selected_candidate(candidates, comparison).map(Sourced::from_candidate),
        reverted: comparison.reverted,
        state: comparison.state,
    }
}

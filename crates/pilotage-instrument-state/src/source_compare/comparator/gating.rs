//! Pure coherence gating and comparison evaluation, free of persistent
//! state. These helpers read candidates, the usable set, and the policy;
//! they never touch the comparator's timers or selection.

use pilotage_alerts::MiscompareFault;

use crate::source_compare::{
    AirframeSourcePolicy, Candidate, Comparable, ComparisonState, IntegrityLevel, MAX_SOURCES,
    SourceEpoch, SourceId, SourceList,
};

/// Indices into the step's candidate slice that survived coherence gating.
pub(super) struct Usable {
    idx: [usize; MAX_SOURCES],
    len: usize,
}

impl Usable {
    pub(super) fn new() -> Self {
        Self {
            idx: [0; MAX_SOURCES],
            len: 0,
        }
    }

    pub(super) fn push(&mut self, i: usize) {
        if self.len < MAX_SOURCES {
            self.idx[self.len] = i;
            self.len += 1;
        }
    }

    pub(super) fn as_slice(&self) -> &[usize] {
        &self.idx[..self.len]
    }
}

/// The instantaneous comparison over the usable set, before hysteresis and
/// persistence turn it into a [`ComparisonState`].
pub(super) enum RawComparison {
    /// Fewer than two usable samples.
    Insufficient,
    /// Two or more usable samples but no compatible, coherent pair.
    NotComparable,
    /// A comparison exists; `max_diff` is the worst pairwise difference after
    /// the accuracy band.
    Compared { max_diff: f32 },
}

/// Whether a sample is a usable simultaneous sample time-wise: its receive
/// and source stamps are not in the future (a future stamp is invalid, never
/// maximally fresh) and its receive age is within the budget.
pub(super) fn is_fresh<M>(candidate: &Candidate<M>, now_ms: u64, max_age_ms: u64) -> bool {
    candidate.receive_time_ms <= now_ms
        && candidate.source_time_ms <= now_ms
        && now_ms - candidate.receive_time_ms <= max_age_ms
}

/// The clock the step tracks: the epoch of the highest-priority candidate
/// that is itself usable — valid, well-formed, and fresh. A stale or
/// future-stamped primary must not anchor the epoch, or it would filter out a
/// fresh, valid secondary on a different epoch and strand the display.
pub(super) fn reference_epoch<M: Comparable>(
    candidates: &[Candidate<M>],
    policy: &AirframeSourcePolicy,
    now_ms: u64,
) -> Option<SourceEpoch> {
    let max_age = policy.max_age_ms();
    for &id in policy.priority().as_slice() {
        if let Some(c) = candidates.iter().find(|c| {
            c.source == id && c.valid && c.measurement.well_formed() && is_fresh(c, now_ms, max_age)
        }) {
            return Some(c.epoch);
        }
    }
    None
}

/// The source ids of the usable candidates.
pub(super) fn available_set<M>(candidates: &[Candidate<M>], usable: &Usable) -> SourceList {
    let mut set = SourceList::new();
    for &i in usable.as_slice() {
        let _ = set.try_push(candidates[i].source);
    }
    set
}

/// The worst pairwise difference among compatible, within-skew pairs, or the
/// reason no such pair exists.
pub(super) fn evaluate_pairs<M: Comparable>(
    candidates: &[Candidate<M>],
    usable: &Usable,
    policy: &AirframeSourcePolicy,
) -> RawComparison {
    let idx = usable.as_slice();
    if idx.len() < 2 {
        return RawComparison::Insufficient;
    }
    let mut any_pair = false;
    let mut max_diff = 0.0f32;
    for (a, &ia) in idx.iter().enumerate() {
        for &ib in idx.iter().skip(a + 1) {
            let ca = &candidates[ia];
            let cb = &candidates[ib];
            if !ca.measurement.datum_compatible(&cb.measurement)
                || ca.source_time_ms.abs_diff(cb.source_time_ms) > policy.skew_budget_ms()
            {
                continue;
            }
            any_pair = true;
            let band = if policy.use_accuracy_band() {
                ca.accuracy.max(0.0) + cb.accuracy.max(0.0)
            } else {
                0.0
            };
            let effective = (ca.measurement.difference(&cb.measurement) - band).max(0.0);
            if effective > max_diff {
                max_diff = effective;
            }
        }
    }
    if any_pair {
        RawComparison::Compared { max_diff }
    } else {
        RawComparison::NotComparable
    }
}

/// The highest-priority source that is available this step.
pub(super) fn first_available_in_priority(
    available: &SourceList,
    policy: &AirframeSourcePolicy,
) -> Option<SourceId> {
    policy
        .priority()
        .as_slice()
        .iter()
        .copied()
        .find(|id| available.contains(*id))
}

/// On a sustained two-source disagreement, a uniquely strictly-higher
/// integrity source may be selected — the only sanctioned way to break the
/// ambiguity. Values are never averaged and the closest-looking source is
/// never preferred.
pub(super) fn apply_integrity_tiebreak<M: Comparable>(
    candidates: &[Candidate<M>],
    usable: &Usable,
    auto: Option<SourceId>,
    state: ComparisonState,
    policy: &AirframeSourcePolicy,
) -> Option<SourceId> {
    if state != ComparisonState::Miscompare || !policy.use_integrity_tiebreak() {
        return auto;
    }
    let chosen_integrity = integrity_of(candidates, usable, auto);
    let mut top = IntegrityLevel::None;
    let mut top_id = None;
    let mut top_count = 0u32;
    for &i in usable.as_slice() {
        let c = &candidates[i];
        if c.integrity > top {
            top = c.integrity;
            top_id = Some(c.source);
            top_count = 1;
        } else if c.integrity == top {
            top_count = top_count.wrapping_add(1);
        }
    }
    match top_id {
        Some(id) if top_count == 1 && top > chosen_integrity => Some(id),
        _ => auto,
    }
}

fn integrity_of<M>(
    candidates: &[Candidate<M>],
    usable: &Usable,
    id: Option<SourceId>,
) -> IntegrityLevel {
    let Some(id) = id else {
        return IntegrityLevel::None;
    };
    usable
        .as_slice()
        .iter()
        .map(|&i| &candidates[i])
        .find(|c| c.source == id)
        .map_or(IntegrityLevel::None, |c| c.integrity)
}

/// The sustained-miscompare fault level for this function.
pub(super) fn fault_level(
    state: ComparisonState,
    function: MiscompareFault,
) -> Option<MiscompareFault> {
    if state == ComparisonState::Miscompare {
        Some(function)
    } else {
        None
    }
}

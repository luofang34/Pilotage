//! The per-function comparison and selection state machine.
//!
//! One [`SourceComparator`] monitors one display function across frames on
//! bounded persistent state alone, reading no interior clock. Every decision
//! is a pure function of that state, the step's candidates, the policy, and
//! the caller-supplied time.

use pilotage_alerts::{AlertCondition, AlertEvent, MiscompareFault};

use crate::source_compare::{
    AirframeSourcePolicy, Candidate, Comparable, ComparisonState, MAX_SOURCES, SourceComparison,
    SourceEpoch, SourceId, SourceList,
};

mod gating;
use gating::RawComparison;
use gating::{
    Usable, apply_integrity_tiebreak, available_set, evaluate_pairs, fault_level,
    first_available_in_priority, is_fresh, reference_epoch,
};

/// Per-source sequence high-water marks, so a replayed or reordered sample
/// (one whose sequence does not advance) is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeqTable {
    entries: [(SourceId, u32); MAX_SOURCES],
    len: usize,
}

impl SeqTable {
    const fn new() -> Self {
        Self {
            entries: [(SourceId(0), 0); MAX_SOURCES],
            len: 0,
        }
    }

    fn last(&self, id: SourceId) -> Option<u32> {
        self.entries[..self.len]
            .iter()
            .find(|(sid, _)| *sid == id)
            .map(|(_, seq)| *seq)
    }

    fn record(&mut self, id: SourceId, seq: u32) {
        for entry in self.entries[..self.len].iter_mut() {
            if entry.0 == id {
                entry.1 = seq;
                return;
            }
        }
        if self.len < MAX_SOURCES {
            self.entries[self.len] = (id, seq);
            self.len += 1;
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// Monitors one display function's candidate sources across frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceComparator {
    function: MiscompareFault,
    selected: Option<SourceId>,
    reverted: bool,
    manual: Option<SourceId>,
    epoch: Option<SourceEpoch>,
    seq: SeqTable,
    disagree_latched: bool,
    disagree_since_ms: Option<u64>,
    primary_healthy_since_ms: Option<u64>,
    state: ComparisonState,
    fault_active: bool,
    generation: u32,
}

impl SourceComparator {
    /// A fresh comparator for one display function, before any samples.
    #[must_use]
    pub fn new(function: MiscompareFault) -> Self {
        Self {
            function,
            selected: None,
            reverted: false,
            manual: None,
            epoch: None,
            seq: SeqTable::new(),
            disagree_latched: false,
            disagree_since_ms: None,
            primary_healthy_since_ms: None,
            state: ComparisonState::InsufficientSources,
            fault_active: false,
            generation: 0,
        }
    }

    /// Sets or clears the pilot's manual source selection. Honored while the
    /// chosen source is available and the policy permits manual selection;
    /// automatic selection resumes when it is unavailable.
    pub fn set_manual(&mut self, selection: Option<SourceId>) {
        self.manual = selection;
    }

    /// The source currently selected for display.
    #[must_use]
    pub fn selected(&self) -> Option<SourceId> {
        self.selected
    }

    /// The current wrapping output generation.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Advances the machine one step and returns the display decision.
    ///
    /// Candidates whose source is not in the policy priority are ignored.
    /// The result is a pure function of `self`, `candidates`, `policy`, and
    /// `now_ms`; the same inputs mutate `self` identically.
    pub fn step<M: Comparable>(
        &mut self,
        candidates: &[Candidate<M>],
        policy: &AirframeSourcePolicy,
        now_ms: u64,
    ) -> SourceComparison {
        let prev = (self.selected, self.reverted, self.state);
        let reference_epoch = reference_epoch(candidates, policy, now_ms);
        self.reset_on_epoch_change(reference_epoch);

        let usable = self.collect_usable(candidates, policy, now_ms);
        let available = available_set(candidates, &usable);
        let raw = evaluate_pairs(candidates, &usable, policy);
        let state = self.update_state(raw, now_ms, policy);
        let manual = self.select(candidates, &usable, &available, state, policy, now_ms);
        self.state = state;
        let transition = self.fault_edge(state);

        if (self.selected, self.reverted, self.state) != prev {
            self.generation = self.generation.wrapping_add(1);
        }
        SourceComparison {
            selected: self.selected,
            state,
            reverted: self.reverted,
            manual,
            fault: fault_level(state, self.function),
            transition,
            generation: self.generation,
        }
    }

    /// Adopts a new reference epoch, resetting every persistence timer so a
    /// restart or clock reset never carries stale sequence, disagreement, or
    /// return state across the boundary.
    fn reset_on_epoch_change(&mut self, reference_epoch: Option<SourceEpoch>) {
        if reference_epoch != self.epoch {
            self.epoch = reference_epoch;
            self.seq.clear();
            self.disagree_latched = false;
            self.disagree_since_ms = None;
            self.primary_healthy_since_ms = None;
        }
    }

    /// Keeps the samples that can serve as simultaneous valid data: declared
    /// valid, well formed, on the reference epoch, fresh, and advancing in
    /// sequence. At most one sample per source (the first in slice order).
    fn collect_usable<M: Comparable>(
        &mut self,
        candidates: &[Candidate<M>],
        policy: &AirframeSourcePolicy,
        now_ms: u64,
    ) -> Usable {
        let mut usable = Usable::new();
        let mut seen = SourceList::new();
        for (i, c) in candidates.iter().enumerate() {
            if !policy.priority().contains(c.source) || seen.contains(c.source) {
                continue;
            }
            if !c.valid || !c.measurement.well_formed() || Some(c.epoch) != self.epoch {
                continue;
            }
            if !is_fresh(c, now_ms, policy.max_age_ms()) {
                continue;
            }
            if self
                .seq
                .last(c.source)
                .is_some_and(|last| c.sequence <= last)
            {
                continue;
            }
            self.seq.record(c.source, c.sequence);
            let _ = seen.try_push(c.source);
            usable.push(i);
        }
        usable
    }

    /// Folds the instantaneous comparison into the four-state result with
    /// magnitude hysteresis (agree/miscompare band) and time persistence, so
    /// jitter cannot chatter and a transient spike never sustains.
    fn update_state(
        &mut self,
        raw: RawComparison,
        now_ms: u64,
        policy: &AirframeSourcePolicy,
    ) -> ComparisonState {
        let max_diff = match raw {
            RawComparison::Insufficient => {
                self.clear_disagreement();
                return ComparisonState::InsufficientSources;
            }
            RawComparison::NotComparable => {
                self.clear_disagreement();
                return ComparisonState::NotComparable;
            }
            RawComparison::Compared { max_diff } => max_diff,
        };
        if self.disagree_latched {
            if max_diff < policy.agree_within() {
                self.disagree_latched = false;
            }
        } else if max_diff >= policy.miscompare_beyond() {
            self.disagree_latched = true;
        }
        if !self.disagree_latched {
            self.disagree_since_ms = None;
            return ComparisonState::Agree;
        }
        let since = *self.disagree_since_ms.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) >= policy.sustain_ms() {
            ComparisonState::Miscompare
        } else {
            ComparisonState::Agree
        }
    }

    fn clear_disagreement(&mut self) {
        self.disagree_latched = false;
        self.disagree_since_ms = None;
    }

    /// Chooses the displayed source. Manual selection wins while available;
    /// otherwise selection follows priority, reverting off a failed primary
    /// and returning to it only after stable availability. Returns whether
    /// the selection is the honored manual one.
    fn select<M: Comparable>(
        &mut self,
        candidates: &[Candidate<M>],
        usable: &Usable,
        available: &SourceList,
        state: ComparisonState,
        policy: &AirframeSourcePolicy,
        now_ms: u64,
    ) -> bool {
        let primary = policy.primary();
        let primary_available = primary.is_some_and(|p| available.contains(p));
        if primary_available {
            self.primary_healthy_since_ms.get_or_insert(now_ms);
        } else {
            self.primary_healthy_since_ms = None;
        }

        if policy.allow_manual()
            && let Some(m) = self.manual
            && available.contains(m)
        {
            self.selected = Some(m);
            self.reverted = Some(m) != primary;
            return true;
        }

        let auto = self.auto_select(available, primary, primary_available, policy, now_ms);
        let chosen = apply_integrity_tiebreak(candidates, usable, auto, state, policy);
        self.selected = chosen;
        self.reverted = match (chosen, primary) {
            (Some(sel), Some(p)) => sel != p,
            (Some(_), None) => true,
            (None, _) => false,
        };
        false
    }

    /// Priority-driven selection with the return-to-primary hysteresis.
    fn auto_select(
        &self,
        available: &SourceList,
        primary: Option<SourceId>,
        primary_available: bool,
        policy: &AirframeSourcePolicy,
        now_ms: u64,
    ) -> Option<SourceId> {
        if primary_available {
            let p = primary?;
            let on_secondary = self.reverted && self.selected.is_some() && self.selected != Some(p);
            if !on_secondary {
                return Some(p);
            }
            let stable = self
                .primary_healthy_since_ms
                .is_some_and(|since| now_ms.saturating_sub(since) >= policy.return_stable_ms());
            if stable {
                Some(p)
            } else {
                match self.selected {
                    Some(cur) if available.contains(cur) => Some(cur),
                    _ => Some(p),
                }
            }
        } else if policy.allow_reversion() {
            first_available_in_priority(available, policy)
        } else {
            None
        }
    }

    /// Emits the ALR-01 transition on the edge of a sustained miscompare.
    fn fault_edge(&mut self, state: ComparisonState) -> Option<AlertEvent> {
        let active = state == ComparisonState::Miscompare;
        if active == self.fault_active {
            return None;
        }
        self.fault_active = active;
        let condition = AlertCondition::Miscompare(self.function);
        Some(if active {
            AlertEvent::Assert(condition)
        } else {
            AlertEvent::Clear(condition)
        })
    }
}

#[cfg(test)]
mod tests;

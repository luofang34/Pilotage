//! Target timing model: prices the counted render work into a worst-case
//! execution time and gates it against a recorded frame deadline.
//!
//! The renderer's work is counted in target-independent units
//! ([`crate::RenderWork`]): worst-case per-edge/segment/disc tests inside
//! coverage evaluations, and source-over composites. A [`TargetTimingModel`]
//! multiplies those counts by per-operation cycle bounds and adds a fixed
//! per-frame overhead (clear + command dispatch), yielding a WCET that is a
//! pure function of the counted work — no wall-clock measurement in CI, so
//! the gate can never flake.
//!
//! There is deliberately no `Default`: a caller must name a model, so a
//! conservative placeholder is never mistaken for a measured target. The one
//! model shipped here is [`CONSERVATIVE_CORTEX_M7_CLASS`], whose provenance
//! is [`CycleProvenance::ConservativeBound`]: no display hardware is
//! selected, and the USB CDC scan (`scripts/detect-target.sh`) found no
//! connected target to measure, so every cycle count is an instruction-level
//! upper bound with the rationale recorded in the timing artifact
//! (`docs/instruments/evidence-artifacts/timing/target-timing.txt`). When a
//! target is measured, a `MeasuredUsbCdc` model replaces the bounds and the
//! deadline tightens to the display requirement; the machinery stays as is.
//!
//! SIM / NOT FOR FLIGHT: the recorded deadline is an anti-regression
//! envelope for the conservative model, not a display-suitability or
//! airworthiness claim.

use crate::report::RenderWork;

/// How a model's cycle counts were obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleProvenance {
    /// Instruction-level upper bounds; no hardware measurement exists.
    ConservativeBound,
    /// Measured on a connected target detected over USB CDC; carries the
    /// reported firmware identity string.
    MeasuredUsbCdc(&'static str),
}

/// Per-target cycle pricing for the counted render work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetTimingModel {
    /// The model's stable name (recorded in the timing artifact).
    pub name: &'static str,
    /// Where the cycle counts came from.
    pub provenance: CycleProvenance,
    /// Core clock the deadline is priced against, hertz.
    pub cpu_hz: u64,
    /// Upper bound on cycles for one edge/segment/disc test, including its
    /// share of the sample-loop overhead.
    pub cycles_per_edge_test: u64,
    /// Upper bound on cycles for one source-over composite, including the
    /// framebuffer read-modify-write.
    pub cycles_per_composite: u64,
    /// Fixed per-frame cycles: the frame clear plus worst-case command
    /// dispatch, independent of scene density.
    pub frame_overhead_cycles: u64,
    /// The frame deadline the WCET is gated against, microseconds.
    pub frame_deadline_us: u64,
}

/// The conservative Cortex-M7-class placeholder model, pending hardware
/// selection and USB CDC measurement.
///
/// Bounds and rationale (recorded in the timing artifact): a winding edge
/// test is integer compares plus two widening multiplies (~20 cycles), a
/// stroke segment distance is short f32 arithmetic with one divide
/// (~40 cycles), and an arc disc test is one `sqrtf` plus compares
/// (~35 cycles) — 48 covers the worst of them plus loop overhead on a
/// zero-wait-state core. A composite is a 4-byte read-modify-write with
/// `div255` blend chains, bounded by 48. The overhead covers the 480x360
/// clear and maximal command dispatch. 480 MHz is the Cortex-M7 class
/// ceiling. All values assume code and framebuffer in zero-wait RAM; flash
/// wait states and cache effects must be re-validated at measurement.
pub const CONSERVATIVE_CORTEX_M7_CLASS: TargetTimingModel = TargetTimingModel {
    name: "conservative-cortex-m7-class",
    provenance: CycleProvenance::ConservativeBound,
    cpu_hz: 480_000_000,
    cycles_per_edge_test: 48,
    cycles_per_composite: 48,
    frame_overhead_cycles: 2_000_000,
    frame_deadline_us: 600_000,
};

impl TargetTimingModel {
    /// Worst-case cycles for one frame of `work`, saturating so an absurd
    /// count can never wrap into a small (plausible) figure.
    #[must_use]
    pub const fn wcet_cycles(&self, work: &RenderWork) -> u64 {
        work.edge_tests
            .saturating_mul(self.cycles_per_edge_test)
            .saturating_add(work.composites.saturating_mul(self.cycles_per_composite))
            .saturating_add(self.frame_overhead_cycles)
    }

    /// Worst-case execution time for one frame of `work`, microseconds,
    /// rounded up so pricing never flatters the work.
    #[must_use]
    pub const fn wcet_us(&self, work: &RenderWork) -> u64 {
        let cycles = self.wcet_cycles(work);
        let per_us = self.cpu_hz / 1_000_000;
        if per_us == 0 {
            return u64::MAX;
        }
        cycles.div_ceil(per_us)
    }

    /// Whether one frame of `work` meets the recorded frame deadline.
    #[must_use]
    pub const fn meets_deadline(&self, work: &RenderWork) -> bool {
        self.wcet_us(work) <= self.frame_deadline_us
    }
}

#[cfg(test)]
mod tests;

//! Target timing model: prices the counted render work, per cost class, into
//! a provisional cost envelope gated against the display-derived frame
//! deadline.
//!
//! The renderer's work is counted in target-independent units
//! ([`crate::RenderWork`]): integer polygon edge tests, f32 stroke segment
//! tests, circle/arc disc tests, arc angular-membership extras, and
//! source-over composites. A [`TargetTimingModel`] multiplies each class by
//! its per-operation cycle bound and adds a fixed per-frame overhead (clear +
//! command dispatch), yielding a cost that is a pure function of the counted
//! work — no wall-clock measurement in CI, so the gate can never flake.
//!
//! **Provisional, not WCET.** Until per-operation cycles are measured on the
//! selected hardware, the derived figure is a *provisional cost envelope*
//! under recorded assumptions (instruction-level bounds with a stated
//! memory-system allowance, an assumed core clock, zero-wait code/framebuffer
//! placement) — never a worst-case-execution-time claim. The provenance is
//! typed: only a [`CycleProvenance::MeasuredUsbCdc`] model, carrying the
//! firmware/build identity, MCU, clock and memory configuration, compiler
//! flags, and the committed raw measurement output, can ground a WCET claim.
//!
//! The frame deadline is not chosen here: it derives from the one recorded
//! display requirement that exists today — the SIM display liveness deadline
//! (`PanelHealth` `livenessDeadlineMs`, 1000 ms, in
//! `clients/web/instrument-health.js`): a panel whose frame generation has
//! not advanced within that deadline fails `LIVENESS`, so producing one frame
//! must fit inside it. When display hardware is selected, the deadline
//! tightens to that display's refresh requirement.
//!
//! There is deliberately no `Default`: a caller must name a model, so a
//! conservative placeholder is never mistaken for a measured target. The one
//! model shipped here is [`CONSERVATIVE_CORTEX_M7_CLASS`]; its bounds,
//! rationale, and derived envelope are recorded in the timing artifact
//! (`docs/instruments/evidence-artifacts/timing/target-timing.txt`), and the
//! USB CDC scan (`scripts/detect-target.sh`) detects a connected target
//! rather than asking.
//!
//! SIM / NOT FOR FLIGHT: nothing here is a display-suitability or
//! airworthiness claim.

use crate::report::RenderWork;

/// How a model's cycle counts were obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleProvenance {
    /// Instruction-level upper bounds with a stated memory-system allowance;
    /// no hardware measurement exists, so derived figures are provisional
    /// envelopes, never WCET claims.
    ConservativeBound,
    /// Measured on a connected target detected over USB CDC. Every field is
    /// required so the measurement is reproducible and auditable; a
    /// measurement that cannot state its configuration is not a measurement.
    MeasuredUsbCdc {
        /// Firmware identity and build hash reported over the CDC handshake.
        firmware: &'static str,
        /// MCU part number.
        mcu: &'static str,
        /// Measured core clock configuration, hertz.
        clock_hz: u64,
        /// Compiler and flags the measured binary was built with.
        compiler: &'static str,
        /// Cache/flash wait-state and memory-placement state during the run.
        memory_state: &'static str,
        /// Repo-relative path of the committed raw measurement output.
        raw_output: &'static str,
    },
}

/// Per-target cycle pricing for the counted render work, one bound per cost
/// class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetTimingModel {
    /// The model's stable name (recorded in the timing artifact).
    pub name: &'static str,
    /// Where the cycle counts came from.
    pub provenance: CycleProvenance,
    /// Core clock the deadline is priced against, hertz. For a
    /// conservative-bound model this is an assumption recorded in the
    /// artifact, not a measured fact.
    pub cpu_hz: u64,
    /// Upper bound on cycles for one integer winding edge test.
    pub cycles_per_polygon_edge_test: u64,
    /// Upper bound on cycles for one f32 capsule segment-distance test.
    pub cycles_per_stroke_segment_test: u64,
    /// Upper bound on cycles for one circle/arc center-distance test.
    pub cycles_per_disc_test: u64,
    /// Upper bound on cycles for one arc angular-membership evaluation (two
    /// cap distances, `atan2f`, `fmodf`).
    pub cycles_per_arc_test: u64,
    /// Upper bound on cycles for one source-over composite, including the
    /// framebuffer read-modify-write.
    pub cycles_per_composite: u64,
    /// Fixed per-frame cycles: the frame clear plus worst-case command
    /// dispatch, independent of scene density.
    pub frame_overhead_cycles: u64,
    /// The frame deadline the envelope is gated against, microseconds —
    /// derived from a recorded display requirement, never invented here.
    pub frame_deadline_us: u64,
}

/// The conservative Cortex-M7-class placeholder model, pending hardware
/// selection and USB CDC measurement.
///
/// Bounds and rationale (recorded in the timing artifact): each bound is a
/// zero-wait instruction-level estimate doubled as a memory-system allowance
/// (flash wait states, cache misses, framebuffer traffic). A winding edge
/// test is integer compares plus two widening multiplies (~20 cycles → 40);
/// a stroke segment distance is short f32 arithmetic with one divide
/// (~40 → 80); a disc test is one `sqrtf` plus compares (~30 → 60); an arc
/// test may add two cap `sqrtf`s, a software `atan2f`, and `fmodf`
/// (~120 → 240); a composite is a 4-byte read-modify-write with `div255`
/// blend chains (~40 → 80). The overhead covers the 480x360 clear and
/// maximal command dispatch. The 480 MHz clock is an assumption (the class
/// ceiling); the envelope scales inversely with the real clock and binds
/// only after measurement. The deadline is the SIM display liveness
/// requirement (`PanelHealth` `livenessDeadlineMs` = 1000 ms).
pub const CONSERVATIVE_CORTEX_M7_CLASS: TargetTimingModel = TargetTimingModel {
    name: "conservative-cortex-m7-class",
    provenance: CycleProvenance::ConservativeBound,
    cpu_hz: 480_000_000,
    cycles_per_polygon_edge_test: 40,
    cycles_per_stroke_segment_test: 80,
    cycles_per_disc_test: 60,
    cycles_per_arc_test: 240,
    cycles_per_composite: 80,
    frame_overhead_cycles: 2_000_000,
    frame_deadline_us: 1_000_000,
};

impl TargetTimingModel {
    /// Provisional cost envelope for one frame of `work`, cycles — the
    /// per-class worst-case sum, saturating so an absurd count can never
    /// wrap into a small (plausible) figure.
    #[must_use]
    pub const fn envelope_cycles(&self, work: &RenderWork) -> u64 {
        work.polygon_edge_tests
            .saturating_mul(self.cycles_per_polygon_edge_test)
            .saturating_add(
                work.stroke_segment_tests
                    .saturating_mul(self.cycles_per_stroke_segment_test),
            )
            .saturating_add(work.disc_tests.saturating_mul(self.cycles_per_disc_test))
            .saturating_add(work.arc_tests.saturating_mul(self.cycles_per_arc_test))
            .saturating_add(work.composites.saturating_mul(self.cycles_per_composite))
            .saturating_add(self.frame_overhead_cycles)
    }

    /// Provisional cost envelope for one frame of `work`, microseconds,
    /// rounded up so pricing never flatters the work.
    #[must_use]
    pub const fn envelope_us(&self, work: &RenderWork) -> u64 {
        let cycles = self.envelope_cycles(work);
        let per_us = self.cpu_hz / 1_000_000;
        if per_us == 0 {
            return u64::MAX;
        }
        cycles.div_ceil(per_us)
    }

    /// Whether one frame of `work` fits the display-derived frame deadline
    /// under this model's assumptions.
    #[must_use]
    pub const fn within_deadline(&self, work: &RenderWork) -> bool {
        self.envelope_us(work) <= self.frame_deadline_us
    }
}

#[cfg(test)]
mod tests;

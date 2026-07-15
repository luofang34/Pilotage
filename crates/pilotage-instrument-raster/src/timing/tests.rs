//! The CI timing gate and the WCET pricing invariants.
#![allow(clippy::expect_used, clippy::panic)]

use super::{CONSERVATIVE_CORTEX_M7_CLASS, CycleProvenance, TargetTimingModel};
use crate::report::RenderWork;

#[test]
fn budget_wcet_meets_the_frame_deadline() {
    // THE timing gate: the engineering work budget, priced by the recorded
    // model, must fit the recorded frame deadline. Growing the budget or the
    // cycle bounds past the deadline is a deterministic CI failure that
    // forces a reasoned artifact update, never a silent regression.
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    assert!(
        model.meets_deadline(&RenderWork::BUDGET),
        "budget WCET {} us exceeds the {} us frame deadline",
        model.wcet_us(&RenderWork::BUDGET),
        model.frame_deadline_us,
    );
}

#[test]
fn wcet_is_monotonic_in_the_counted_work() {
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    let base = RenderWork {
        coverage_samples: 10,
        edge_tests: 100,
        composites: 50,
    };
    let more_edges = RenderWork {
        edge_tests: 101,
        ..base
    };
    let more_composites = RenderWork {
        composites: 51,
        ..base
    };
    assert!(model.wcet_cycles(&more_edges) > model.wcet_cycles(&base));
    assert!(model.wcet_cycles(&more_composites) > model.wcet_cycles(&base));
}

#[test]
fn wcet_saturates_instead_of_wrapping() {
    // An absurd count must never wrap into a small, plausible-looking WCET.
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    let absurd = RenderWork {
        coverage_samples: u64::MAX,
        edge_tests: u64::MAX,
        composites: u64::MAX,
    };
    assert_eq!(model.wcet_cycles(&absurd), u64::MAX);
    assert!(!model.meets_deadline(&absurd));
}

#[test]
fn a_sub_megahertz_clock_prices_to_the_failing_extreme() {
    // cpu_hz below 1 MHz has no whole cycles-per-microsecond figure; the
    // pricing fails closed to u64::MAX rather than dividing by zero.
    let model = TargetTimingModel {
        cpu_hz: 999_999,
        ..CONSERVATIVE_CORTEX_M7_CLASS
    };
    assert_eq!(model.wcet_us(&RenderWork::BUDGET), u64::MAX);
    assert!(!model.meets_deadline(&RenderWork::BUDGET));
}

#[test]
fn the_shipped_model_is_a_conservative_bound_not_a_measurement() {
    // Until a target is detected over USB CDC and measured, the shipped
    // model must say so; a measured model carries the firmware identity.
    assert_eq!(
        CONSERVATIVE_CORTEX_M7_CLASS.provenance,
        CycleProvenance::ConservativeBound
    );
}

/// The value of a `<field>: <value>` line in the timing artifact.
fn artifact_field(text: &str, field: &str) -> std::string::String {
    let key = std::format!("{field}:");
    text.lines()
        .find_map(|line| line.trim().strip_prefix(&key))
        .unwrap_or_else(|| panic!("timing artifact has no {field} field"))
        .trim()
        .into()
}

#[test]
fn the_timing_artifact_matches_the_shipped_model() {
    // The committed timing artifact is the reviewable record of the model;
    // this guard fails CI when either side drifts, so the recorded bounds,
    // budget, WCET, and deadline can never disagree with the shipped code.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/instruments/evidence-artifacts/timing/target-timing.txt");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    let budget = RenderWork::BUDGET;
    assert_eq!(artifact_field(&text, "target"), model.name);
    assert_eq!(artifact_field(&text, "provenance"), "conservative-bound");
    let numbers = [
        ("cpu-hz", model.cpu_hz),
        ("cycles-per-edge-test", model.cycles_per_edge_test),
        ("cycles-per-composite", model.cycles_per_composite),
        ("frame-overhead-cycles", model.frame_overhead_cycles),
        ("budget-coverage-samples", budget.coverage_samples),
        ("budget-edge-tests", budget.edge_tests),
        ("budget-composites", budget.composites),
        ("wcet-cycles", model.wcet_cycles(&budget)),
        ("wcet-us", model.wcet_us(&budget)),
        ("frame-deadline-us", model.frame_deadline_us),
    ];
    for (field, expected) in numbers {
        assert_eq!(
            artifact_field(&text, field),
            std::format!("{expected}"),
            "timing artifact field {field} disagrees with the shipped model"
        );
    }
}

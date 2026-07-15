//! The CI envelope gate and the pricing invariants.
#![allow(clippy::expect_used, clippy::panic)]

use super::{CONSERVATIVE_CORTEX_M7_CLASS, CycleProvenance, TargetTimingModel};
use crate::report::RenderWork;

#[test]
fn budget_envelope_fits_the_display_derived_deadline() {
    // THE timing gate: the per-class work budget, priced by the recorded
    // model, must fit the frame deadline derived from the SIM display
    // liveness requirement. Growing a budget or a cycle bound past it is a
    // deterministic CI failure that forces a reasoned artifact update,
    // never a silent regression.
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    assert!(
        model.within_deadline(&RenderWork::BUDGET),
        "budget envelope {} us exceeds the {} us frame deadline",
        model.envelope_us(&RenderWork::BUDGET),
        model.frame_deadline_us,
    );
}

#[test]
fn the_envelope_is_monotonic_in_every_cost_class() {
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    let base = RenderWork {
        coverage_samples: 10,
        polygon_edge_tests: 100,
        stroke_segment_tests: 100,
        disc_tests: 100,
        arc_tests: 100,
        composites: 50,
    };
    let bumped = [
        RenderWork {
            polygon_edge_tests: 101,
            ..base
        },
        RenderWork {
            stroke_segment_tests: 101,
            ..base
        },
        RenderWork {
            disc_tests: 101,
            ..base
        },
        RenderWork {
            arc_tests: 101,
            ..base
        },
        RenderWork {
            composites: 51,
            ..base
        },
    ];
    for work in bumped {
        assert!(model.envelope_cycles(&work) > model.envelope_cycles(&base));
    }
}

#[test]
fn an_arc_test_is_priced_dearer_than_a_disc_test() {
    // The review counterexample: an arc sample performs cap distances,
    // atan2f, and fmodf beyond its disc test, so pricing an arc like a bare
    // disc under-counts. The model must keep the arc class strictly dearer.
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    assert!(model.cycles_per_arc_test > model.cycles_per_disc_test);
    let disc_only = RenderWork {
        disc_tests: 1_000,
        ..RenderWork::default()
    };
    let arc_heavy = RenderWork {
        disc_tests: 1_000,
        arc_tests: 1_000,
        ..RenderWork::default()
    };
    assert!(model.envelope_cycles(&arc_heavy) > model.envelope_cycles(&disc_only));
}

#[test]
fn the_envelope_saturates_instead_of_wrapping() {
    // An absurd count must never wrap into a small, plausible-looking figure.
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    let absurd = RenderWork {
        coverage_samples: u64::MAX,
        polygon_edge_tests: u64::MAX,
        stroke_segment_tests: u64::MAX,
        disc_tests: u64::MAX,
        arc_tests: u64::MAX,
        composites: u64::MAX,
    };
    assert_eq!(model.envelope_cycles(&absurd), u64::MAX);
    assert!(!model.within_deadline(&absurd));
}

#[test]
fn a_sub_megahertz_clock_prices_to_the_failing_extreme() {
    // cpu_hz below 1 MHz has no whole cycles-per-microsecond figure; the
    // pricing fails closed to u64::MAX rather than dividing by zero.
    let model = TargetTimingModel {
        cpu_hz: 999_999,
        ..CONSERVATIVE_CORTEX_M7_CLASS
    };
    assert_eq!(model.envelope_us(&RenderWork::BUDGET), u64::MAX);
    assert!(!model.within_deadline(&RenderWork::BUDGET));
}

#[test]
fn the_shipped_model_is_a_conservative_bound_not_a_measurement() {
    // Until a target is detected over USB CDC and measured — with its
    // firmware/build identity, MCU, clock and memory configuration, compiler
    // flags, and raw output recorded — the shipped model must say so.
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
    // budgets, envelope, and deadline can never disagree with the shipped
    // code.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/instruments/evidence-artifacts/timing/target-timing.txt");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let model = CONSERVATIVE_CORTEX_M7_CLASS;
    let budget = RenderWork::BUDGET;
    assert_eq!(artifact_field(&text, "target"), model.name);
    assert_eq!(artifact_field(&text, "provenance"), "conservative-bound");
    let numbers = [
        ("assumed-cpu-hz", model.cpu_hz),
        (
            "cycles-per-polygon-edge-test",
            model.cycles_per_polygon_edge_test,
        ),
        (
            "cycles-per-stroke-segment-test",
            model.cycles_per_stroke_segment_test,
        ),
        ("cycles-per-disc-test", model.cycles_per_disc_test),
        ("cycles-per-arc-test", model.cycles_per_arc_test),
        ("cycles-per-composite", model.cycles_per_composite),
        ("frame-overhead-cycles", model.frame_overhead_cycles),
        ("budget-coverage-samples", budget.coverage_samples),
        ("budget-polygon-edge-tests", budget.polygon_edge_tests),
        ("budget-stroke-segment-tests", budget.stroke_segment_tests),
        ("budget-disc-tests", budget.disc_tests),
        ("budget-arc-tests", budget.arc_tests),
        ("budget-composites", budget.composites),
        ("envelope-cycles", model.envelope_cycles(&budget)),
        ("envelope-us", model.envelope_us(&budget)),
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

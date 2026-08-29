#![allow(clippy::expect_used, clippy::panic)]

use super::{
    EvaluatorClass, EvaluatorSourceEntry, GATE_IMPLEMENTATION_ID, METRIC_IMPLEMENTATION_ID,
    embedded_document, read_back_sources, validate_entries,
};

#[test]
fn each_embedded_inventory_matches_its_embedded_digest() {
    for class in [EvaluatorClass::Metric, EvaluatorClass::Gate] {
        read_back_sources(class).expect("the build inventory reads back");
    }
}

#[test]
fn the_metric_inventory_names_every_angular_and_timing_source() {
    let entries = read_back_sources(EvaluatorClass::Metric).expect("metric inventory");
    let paths = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();

    for expected in [
        "crates/pilotage-flight-quality/src/angular.rs",
        "crates/pilotage-flight-quality/src/angular_release.rs",
        "crates/pilotage-flight-quality/src/collective.rs",
        "crates/pilotage-flight-quality/src/response.rs",
        "crates/pilotage-flight-quality/src/series.rs",
        "crates/pilotage-flight-quality/src/vocabulary.rs",
    ] {
        assert!(paths.contains(&expected), "{expected} is not bound");
    }
}

#[test]
fn the_gate_inventory_holds_no_metric_only_source() {
    let entries = read_back_sources(EvaluatorClass::Gate).expect("gate inventory");
    let paths = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();

    assert!(paths.contains(&"crates/pilotage-flight-quality/src/gate.rs"));
    assert!(!paths.contains(&"crates/pilotage-flight-quality/src/angular.rs"));
    assert!(!paths.contains(&"tools/flight-tune/src/flight_quality/metrics.rs"));
}

#[test]
fn one_inventory_cannot_stand_in_for_the_other() {
    assert_ne!(
        embedded_document(EvaluatorClass::Metric),
        embedded_document(EvaluatorClass::Gate)
    );
    assert_ne!(METRIC_IMPLEMENTATION_ID, GATE_IMPLEMENTATION_ID);

    let metric = read_back_sources(EvaluatorClass::Metric).expect("metric inventory");
    let gates = read_back_sources(EvaluatorClass::Gate).expect("gate inventory");
    assert_ne!(metric, gates);
}

#[test]
fn a_test_source_cannot_enter_a_production_identity() {
    for path in [
        "tools/flight-tune/src/flight_quality/tests.rs",
        "tools/flight-tune/src/flight_quality/tests/metrics.rs",
        "crates/pilotage-flight-quality/src/angular_tests.rs",
        "crates/pilotage-flight-quality/src/test_trace.rs",
    ] {
        let error = validate_entries(&[entry(path)]).expect_err("a test path is refused");
        assert!(
            error.to_string().contains("test source"),
            "{path} was refused for another reason: {error}"
        );
    }
}

#[test]
fn a_source_outside_the_owned_roots_is_refused() {
    for path in [
        "tools/flight-tune/src/journal.rs",
        "crates/pilotage-trial/src/lib.rs",
        "crates/pilotage-flight-quality/src/../../../etc/passwd.rs",
    ] {
        let error = validate_entries(&[entry(path)]).expect_err("an unowned path is refused");
        assert!(
            error.to_string().contains("owned source"),
            "{path} was refused for another reason: {error}"
        );
    }
}

#[test]
fn a_repeated_path_is_refused() {
    let path = "crates/pilotage-flight-quality/src/angular.rs";
    let error =
        validate_entries(&[entry(path), entry(path)]).expect_err("a repeated path is refused");

    assert!(error.to_string().contains("repeats a path"));
}

#[test]
fn an_unordered_inventory_is_refused() {
    let entries = [
        entry("crates/pilotage-flight-quality/src/control.rs"),
        entry("crates/pilotage-flight-quality/src/angular.rs"),
    ];
    let error = validate_entries(&entries).expect_err("an unordered inventory is refused");

    assert!(error.to_string().contains("canonical path order"));
}

#[test]
fn an_entry_without_a_content_identity_is_refused() {
    let mut short = entry("crates/pilotage-flight-quality/src/angular.rs");
    short.sha256.truncate(63);
    let mut empty = entry("crates/pilotage-flight-quality/src/angular.rs");
    empty.bytes = 0;

    for candidate in [short, empty] {
        let error = validate_entries(&[candidate]).expect_err("an empty entry is refused");
        assert!(error.to_string().contains("no content identity"));
    }
}

#[test]
fn an_empty_inventory_is_refused() {
    let error = validate_entries(&[]).expect_err("an empty inventory is refused");

    assert!(error.to_string().contains("is empty"));
}

fn entry(path: &str) -> EvaluatorSourceEntry {
    EvaluatorSourceEntry {
        path: path.to_owned(),
        sha256: "0".repeat(64),
        bytes: 1,
    }
}

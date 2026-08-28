#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::classify;
use super::model::{ClassifierModel, Domain, dependency_closure};

fn synthetic_model() -> ClassifierModel {
    let mut dependencies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    dependencies.insert("bridge".to_owned(), BTreeSet::from(["runtime".to_owned()]));
    dependencies.insert("runtime".to_owned(), BTreeSet::new());
    dependencies.insert("tuner".to_owned(), BTreeSet::new());
    let closure = dependency_closure(&["bridge".to_owned()], &dependencies);
    let domains = BTreeMap::from([(
        "apple".to_owned(),
        Domain {
            paths: vec!["clients/apple/".to_owned()],
            extra_paths: vec!["tools/hid-probe/fixtures/".to_owned()],
            package_closure: closure,
        },
    )]);
    ClassifierModel::synthetic(
        vec![
            ("bridge".to_owned(), PathBuf::from("clients/bridge")),
            ("runtime".to_owned(), PathBuf::from("crates/runtime")),
            ("tuner".to_owned(), PathBuf::from("tools/tuner")),
        ],
        vec!["docs/".to_owned(), "*.md".to_owned()],
        domains,
    )
}

fn one(file: &str) -> Vec<String> {
    vec![file.to_owned()]
}

#[test]
fn a_domain_path_answers_that_domain() {
    let outcome = classify(&one("clients/apple/App/Main.swift"), &synthetic_model());
    assert!(!outcome.everything);
    assert_eq!(outcome.domains.get("apple"), Some(&true));
}

/// The gap independent review found in the path-only classifier: golden
/// fixtures consumed by another domain's tests. Declared extra paths
/// must answer the domain that reads them.
#[test]
fn a_declared_fixture_answers_its_consumer() {
    let outcome = classify(
        &one("tools/hid-probe/fixtures/capture.json"),
        &synthetic_model(),
    );
    assert!(!outcome.everything);
    assert_eq!(outcome.domains.get("apple"), Some(&true));
}

#[test]
fn a_dependency_of_a_domain_root_answers_the_domain() {
    let outcome = classify(&one("crates/runtime/src/lib.rs"), &synthetic_model());
    assert!(!outcome.everything);
    assert_eq!(outcome.domains.get("apple"), Some(&true));
}

#[test]
fn a_package_outside_the_closure_stays_quiet() {
    let outcome = classify(&one("tools/tuner/src/engine.rs"), &synthetic_model());
    assert!(!outcome.everything);
    assert_eq!(outcome.domains.get("apple"), Some(&false));
}

#[test]
fn inert_paths_affect_nothing() {
    for file in ["docs/design.md", "README.md"] {
        let outcome = classify(&one(file), &synthetic_model());
        assert!(!outcome.everything, "{file}");
        assert_eq!(outcome.domains.get("apple"), Some(&false), "{file}");
    }
}

/// Fail open: a file no declaration places runs everything.
#[test]
fn an_unplaced_file_runs_everything() {
    let outcome = classify(&one("mystery/artifact.bin"), &synthetic_model());
    assert!(outcome.everything);
}

/// The classifier distrusts changes to its own inputs.
#[test]
fn classifier_inputs_run_everything() {
    for file in [
        "tools/xtask/src/affected.rs",
        ".github/workflows/ci.yml",
        "Cargo.lock",
        "Cargo.toml",
        "tools/tuner/Cargo.toml",
    ] {
        let outcome = classify(&one(file), &synthetic_model());
        assert!(outcome.everything, "{file}");
    }
}

/// The answers hold against the REAL workspace graph, so a refactor
/// that breaks a declared relationship fails here rather than silently
/// mis-scoping CI.
#[test]
fn real_workspace_pins_the_apple_closure() {
    let metadata = super::load_metadata().expect("cargo metadata");
    let model = ClassifierModel::from_workspace(&metadata).expect("model");

    let fixtures = classify(&one("tools/hid-probe/fixtures/apple-capture.json"), &model);
    assert_eq!(fixtures.domains.get("apple"), Some(&true));
    assert!(!fixtures.everything);

    let bridge_dep = classify(
        &one("crates/pilotage-instrument-runtime/src/lib.rs"),
        &model,
    );
    assert_eq!(
        bridge_dep.domains.get("apple"),
        Some(&true),
        "the bridge depends on the instrument runtime"
    );

    let tuner = classify(&one("tools/flight-tune/src/engine.rs"), &model);
    assert_eq!(tuner.domains.get("apple"), Some(&false));
    assert!(!tuner.everything);
}

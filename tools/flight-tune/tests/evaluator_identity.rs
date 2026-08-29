//! The complete production-input identity of the flight-quality evaluators.
//!
//! These tests run the same inventory code the build script runs, against
//! copies of the real workspace sources. A guard that only read the embedded
//! constant could not tell a stale constant from a live one; a guard that runs
//! the inventory over a tree it can change can.
//!
//! SIM / NOT FOR FLIGHT.

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

// The build script is the only production caller, so parts of the shared
// inventory module are unused in this test crate.
#[path = "../build_support/evaluator_source_identity.rs"]
#[allow(dead_code)]
mod evaluator_source_identity;

use evaluator_source_identity::{
    EvaluatorKind, GATE_PRODUCTION_INPUTS, METRIC_PRODUCTION_INPUTS, calculate, digest_document,
    digest_named_bytes,
};

/// The metric source that measures yaw and angular step response.
const ANGULAR_SOURCE: &str = "crates/pilotage-flight-quality/src/angular.rs";

/// The metric source that measures delay, rise, and settling windows.
///
/// The timing windows and the event markers that bound them live with the step
/// response rules rather than in a module of their own.
const TIMING_SOURCE: &str = "crates/pilotage-flight-quality/src/response.rs";

/// One copy of the workspace evaluator sources that a test can change.
struct SourceTree {
    root: PathBuf,
}

impl SourceTree {
    fn copy() -> Self {
        let root = std::env::temp_dir().join(format!(
            "pilotage-468-evaluator-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).expect("clear the previous source tree");
        }
        let workspace = workspace_root();
        for relative in [
            "tools/flight-tune/build.rs",
            "tools/flight-tune/build_support/evaluator_source_identity.rs",
            "tools/flight-tune/src/flight_quality.rs",
        ] {
            copy_file(&workspace, &root, Path::new(relative));
        }
        for relative in [
            "tools/flight-tune/src/flight_quality",
            "crates/pilotage-flight-quality/src",
        ] {
            copy_directory(&workspace.join(relative), &root.join(relative));
        }
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn digest(&self, kind: EvaluatorKind) -> [u8; 32] {
        calculate(&self.root, kind)
            .expect("calculate the evaluator source identity")
            .digest
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create a source directory");
        }
        fs::write(path, contents).expect("write a source");
    }

    fn remove(&self, relative: &str) {
        fs::remove_file(self.root.join(relative)).expect("remove a source");
    }

    fn append(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        let mut contents = fs::read_to_string(&path).expect("read a source");
        contents.push_str(text);
        fs::write(path, contents).expect("rewrite a source");
    }
}

impl Drop for SourceTree {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).ok();
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_file(from_root: &Path, to_root: &Path, relative: &Path) {
    let target = to_root.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).expect("create a source directory");
    }
    fs::copy(from_root.join(relative), target).expect("copy a source");
}

fn copy_directory(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create a source directory");
    for entry in fs::read_dir(from).expect("read a source directory") {
        let path = entry.expect("read a source entry").path();
        let name = path.file_name().expect("name a source entry");
        if path.is_dir() {
            copy_directory(&path, &to.join(name));
        } else {
            fs::copy(&path, to.join(name)).expect("copy a source");
        }
    }
}

fn named_bytes(inputs: &[&str]) -> Vec<(String, Vec<u8>)> {
    inputs
        .iter()
        .map(|name| {
            (
                (*name).to_owned(),
                format!("contents of {name}").into_bytes(),
            )
        })
        .collect()
}

#[test]
fn the_metric_inventory_names_every_production_metric_source_once() {
    let tree = SourceTree::copy();

    let inventory = calculate(tree.path(), EvaluatorKind::Metric).expect("metric inventory");

    assert_eq!(inventory.names.len(), METRIC_PRODUCTION_INPUTS.len());
    let unique = inventory.names.iter().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), inventory.names.len());
    assert!(inventory.names.iter().any(|name| name == ANGULAR_SOURCE));
    assert!(inventory.names.iter().any(|name| name == TIMING_SOURCE));
}

#[test]
fn a_change_to_the_angular_source_changes_only_the_metric_identity() {
    let tree = SourceTree::copy();
    let metric = tree.digest(EvaluatorKind::Metric);
    let gates = tree.digest(EvaluatorKind::Gate);

    tree.append(ANGULAR_SOURCE, "\n// one changed angular rule\n");

    assert_ne!(tree.digest(EvaluatorKind::Metric), metric);
    assert_eq!(tree.digest(EvaluatorKind::Gate), gates);
}

#[test]
fn a_change_to_the_timing_source_changes_only_the_metric_identity() {
    let tree = SourceTree::copy();
    let metric = tree.digest(EvaluatorKind::Metric);
    let gates = tree.digest(EvaluatorKind::Gate);

    tree.append(TIMING_SOURCE, "\n// one changed settling window\n");

    assert_ne!(tree.digest(EvaluatorKind::Metric), metric);
    assert_eq!(tree.digest(EvaluatorKind::Gate), gates);
}

#[test]
fn every_declared_production_source_changes_its_evaluator_identity() {
    for (kind, inputs) in [
        (EvaluatorKind::Metric, METRIC_PRODUCTION_INPUTS.as_slice()),
        (EvaluatorKind::Gate, GATE_PRODUCTION_INPUTS.as_slice()),
    ] {
        for input in inputs {
            let tree = SourceTree::copy();
            let before = tree.digest(kind);

            tree.append(input, "\n// one changed production rule\n");

            assert_ne!(tree.digest(kind), before, "{input} does not bind {kind:?}");
        }
    }
}

#[test]
fn a_new_production_metric_source_fails_the_inventory_guard() {
    let tree = SourceTree::copy();
    tree.digest(EvaluatorKind::Metric);

    tree.write(
        "tools/flight-tune/src/flight_quality/overshoot.rs",
        "//! One unbound production metric rule.\n",
    );

    let Err(error) = calculate(tree.path(), EvaluatorKind::Metric) else {
        panic!("an extra source is refused");
    };
    let detail = error.to_string();
    assert!(detail.contains("extra"), "{detail}");
    assert!(detail.contains("overshoot.rs"), "{detail}");
}

#[test]
fn a_missing_production_metric_source_fails_the_inventory_guard() {
    let tree = SourceTree::copy();
    tree.digest(EvaluatorKind::Metric);

    tree.remove(ANGULAR_SOURCE);

    let Err(error) = calculate(tree.path(), EvaluatorKind::Metric) else {
        panic!("a missing source is refused");
    };
    let detail = error.to_string();
    assert!(detail.contains("missing"), "{detail}");
    assert!(detail.contains("angular.rs"), "{detail}");
}

#[test]
fn a_test_source_does_not_enter_a_production_identity() {
    let tree = SourceTree::copy();
    let metric = tree.digest(EvaluatorKind::Metric);
    let gates = tree.digest(EvaluatorKind::Gate);

    tree.write(
        "crates/pilotage-flight-quality/src/overshoot_tests.rs",
        "//! One added unit test.\n",
    );
    tree.write(
        "tools/flight-tune/src/flight_quality/tests/overshoot.rs",
        "//! One added test module.\n",
    );
    tree.write(
        "tools/flight-tune/src/flight_quality/test_support.rs",
        "//! One added test helper.\n",
    );
    tree.append(
        "tools/flight-tune/src/flight_quality/tests.rs",
        "\n// one changed assertion\n",
    );

    assert_eq!(tree.digest(EvaluatorKind::Metric), metric);
    assert_eq!(tree.digest(EvaluatorKind::Gate), gates);
}

#[test]
fn input_order_cannot_change_the_canonical_digest() {
    for (kind, inputs) in [
        (EvaluatorKind::Metric, METRIC_PRODUCTION_INPUTS.as_slice()),
        (EvaluatorKind::Gate, GATE_PRODUCTION_INPUTS.as_slice()),
    ] {
        let sorted = named_bytes(inputs);
        let mut reversed = sorted.clone();
        reversed.reverse();

        assert_eq!(
            digest_named_bytes(kind, &sorted).expect("sorted digest"),
            digest_named_bytes(kind, &reversed).expect("reversed digest")
        );
    }
}

#[test]
fn the_metric_and_gate_inventories_cannot_substitute_for_each_other() {
    let metric = named_bytes(METRIC_PRODUCTION_INPUTS.as_slice());
    let gates = named_bytes(GATE_PRODUCTION_INPUTS.as_slice());

    assert_ne!(
        digest_named_bytes(EvaluatorKind::Metric, &metric).expect("metric digest"),
        digest_named_bytes(EvaluatorKind::Gate, &gates).expect("gate digest")
    );
    digest_named_bytes(EvaluatorKind::Metric, &gates).expect_err("the gate inventory is refused");
    digest_named_bytes(EvaluatorKind::Gate, &metric).expect_err("the metric inventory is refused");
}

#[test]
fn one_evaluator_document_cannot_be_read_as_the_other() {
    let tree = SourceTree::copy();
    let metric = calculate(tree.path(), EvaluatorKind::Metric).expect("metric inventory");

    digest_document(EvaluatorKind::Metric, &metric.document).expect("the metric document reads");
    digest_document(EvaluatorKind::Gate, &metric.document)
        .expect_err("the metric document is not a gate document");
}

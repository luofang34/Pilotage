//! The complete production-input identity of the Aviate scenario runtime.
//!
//! These tests use the same inventory code the build script runs, against
//! copies of the real package sources. A guard that only read the embedded
//! constant could not tell a stale constant from a live one; a guard that
//! runs the inventory over a tree it can change can.
//!
//! SIM / NOT FOR FLIGHT.

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use flight_tune_aviate::direct_transport::direct_transport_sources;

// The build script is the only production caller, so parts of the shared
// inventory module are unused in this test crate.
#[path = "../build_support/runtime_source_identity.rs"]
#[allow(dead_code)]
mod runtime_source_identity;

use runtime_source_identity::{
    PRODUCTION_INPUTS, calculate, digest_named_bytes, readback_document,
};

/// One copy of the package's runtime sources that a test can change.
struct SourceTree {
    root: PathBuf,
}

impl SourceTree {
    fn copy() -> Self {
        let root = std::env::temp_dir().join(format!(
            "pilotage-469-runtime-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).expect("clear the previous source tree");
        }
        fs::create_dir_all(root.join("src")).expect("create the source tree");
        let package = package_root();
        copy_file(&package, &root, Path::new("src/runtime.rs"));
        copy_directory(&package.join("src/runtime"), &root.join("src/runtime"));
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn digest(&self) -> [u8; 32] {
        calculate(&self.root)
            .expect("calculate the runtime source identity")
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

fn package_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

#[test]
fn the_inventory_names_every_production_runtime_source_once() {
    let tree = SourceTree::copy();
    let inventory = calculate(tree.path()).expect("calculate the inventory");
    assert_eq!(inventory.names.len(), PRODUCTION_INPUTS.len());
    let unique = inventory.names.iter().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), inventory.names.len());
    let entries = readback_document(&inventory.document).expect("read the inventory back");
    assert_eq!(entries.len(), PRODUCTION_INPUTS.len());
    assert!(
        entries
            .iter()
            .all(|(_, sha256, bytes)| { sha256.len() == 64 && *bytes > 0 })
    );
}

#[test]
fn a_change_to_timing_changes_the_identity() {
    let tree = SourceTree::copy();
    let before = tree.digest();
    tree.append(
        "src/runtime/timing.rs",
        "\n// one changed production byte\n",
    );
    assert_ne!(tree.digest(), before);
}

#[test]
fn a_change_to_the_stimulus_waveform_changes_the_identity() {
    let tree = SourceTree::copy();
    let before = tree.digest();
    tree.append(
        "src/runtime/phase/waveform.rs",
        "\n// one changed production byte\n",
    );
    assert_ne!(tree.digest(), before);
}

#[test]
fn a_new_production_runtime_source_fails_the_inventory_guard() {
    let tree = SourceTree::copy();
    assert!(tree.digest() != [0; 32]);
    tree.write(
        "src/runtime/unbound.rs",
        "//! An unbound production source.\n",
    );
    let detail = guard_refusal(tree.path(), "a new production source");
    assert!(detail.contains("differs"), "{detail}");
    assert!(detail.contains("unbound.rs"), "{detail}");
}

#[test]
fn a_missing_production_runtime_source_fails_the_inventory_guard() {
    let tree = SourceTree::copy();
    tree.remove("src/runtime/phase/waveform.rs");
    let detail = guard_refusal(tree.path(), "a missing production source");
    assert!(detail.contains("differs"), "{detail}");
    assert!(detail.contains("waveform.rs"), "{detail}");
}

#[test]
fn a_test_source_does_not_enter_the_production_identity() {
    let tree = SourceTree::copy();
    let before = tree.digest();
    tree.write("src/runtime/tests.rs", "//! Unit tests.\n");
    tree.write("src/runtime/tests/support.rs", "//! Test support.\n");
    tree.write("src/runtime/phase/waveform_tests.rs", "//! More tests.\n");
    tree.write("src/runtime/phase/tests/schedule.rs", "//! More tests.\n");
    assert_eq!(
        tree.digest(),
        before,
        "a test source must not change the production identity"
    );
}

/// One synthetic byte set for the complete declared inventory.
fn declared_inputs() -> Vec<(String, Vec<u8>)> {
    PRODUCTION_INPUTS
        .iter()
        .enumerate()
        .map(|(index, name)| ((*name).to_owned(), format!("source {index}").into_bytes()))
        .collect()
}

#[test]
fn input_ordering_cannot_change_the_canonical_digest() {
    let ordered = declared_inputs();
    let mut reversed = ordered.clone();
    reversed.reverse();
    let mut rotated = ordered.clone();
    rotated.rotate_left(7);
    let first = digest_named_bytes(&ordered).expect("digest the ordered inputs");
    assert_eq!(
        digest_named_bytes(&reversed).expect("digest the reversed inputs"),
        first
    );
    assert_eq!(
        digest_named_bytes(&rotated).expect("digest the rotated inputs"),
        first
    );

    let mut changed = ordered;
    changed[3].1 = b"one changed production source".to_vec();
    assert_ne!(
        digest_named_bytes(&changed).expect("digest the changed inputs"),
        first
    );
}

/// The refusal detail the inventory guard reports, or a failed test.
///
/// [`calculate`] returns an inventory that states no [`std::fmt::Debug`],
/// so the refusal is read by matching rather than by unwrapping.
fn guard_refusal(root: &Path, expected: &str) -> String {
    match calculate(root) {
        Ok(_) => panic!("the inventory guard accepted {expected}"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn a_repeated_inventory_path_fails_the_canonical_document() {
    let mut repeated = declared_inputs();
    let first = repeated[0].clone();
    repeated.push((first.0, b"a second copy of one path".to_vec()));
    match digest_named_bytes(&repeated) {
        Ok(_) => panic!("the canonical document accepted a repeated path"),
        Err(error) => assert!(error.to_string().contains("repeated path"), "{error}"),
    }
}

#[test]
fn every_direct_step_source_is_bound_to_the_transport_identity() {
    let bound = direct_transport_sources()
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut present = BTreeSet::new();
    present.insert("direct_transport.rs".to_owned());
    collect_production_names(
        &package_root().join("src/direct_transport"),
        "direct_transport",
        &mut present,
    );
    assert_eq!(
        bound, present,
        "every direct-step production source must enter the transport identity"
    );
}

fn collect_production_names(directory: &Path, prefix: &str, names: &mut BTreeSet<String>) {
    for entry in fs::read_dir(directory).expect("read a direct-transport directory") {
        let path = entry.expect("read a direct-transport entry").path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("name a direct-transport entry")
            .to_owned();
        if path.is_dir() {
            if name != "tests" {
                collect_production_names(&path, &format!("{prefix}/{name}"), names);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            names.insert(format!("{prefix}/{name}"));
        }
    }
}

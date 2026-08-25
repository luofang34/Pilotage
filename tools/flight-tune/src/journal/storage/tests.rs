#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pilotage_durable_storage::{DurableStore, ExpectedValue, ObjectName};

use super::layout::LAYOUT_MARKER;
use super::{MAX_DOCUMENT_BYTES, encode, exact_head, open};
use crate::{Digest, TuneError};

#[path = "tests/prospective.rs"]
mod prospective;

#[test]
fn an_oversized_document_is_rejected_before_a_write() {
    let content = "x".repeat(MAX_DOCUMENT_BYTES);
    let error = encode("oversized test document", &content).expect_err("reject oversized data");

    assert!(matches!(error, TuneError::DocumentTooLarge { .. }));
}

#[test]
fn legacy_evidence_is_rejected_without_a_catalog_change() {
    let directory = TestDirectory::with_root("legacy-layout");
    write_private(&directory.path().join("HEAD.json"), b"old-head");
    write_private(&directory.path().join("legacy-sentinel"), b"old-evidence");
    let before = catalog(directory.path());

    let error = open(directory.path()).err().expect("reject legacy layout");

    assert_invalid_layout(error);
    assert_eq!(catalog(directory.path()), before);
    assert!(!directory.path().join(".pilotage-writer-lock").exists());
    assert!(!directory.path().join(LAYOUT_MARKER).exists());
}

#[test]
fn an_empty_root_bootstraps_the_versioned_layout() {
    let directory = TestDirectory::with_root("empty-bootstrap");

    let opened = open(directory.path()).expect("bootstrap empty root");
    drop(opened);

    assert_bootstrap_objects(directory.path());
}

#[test]
fn an_exact_lock_only_root_resumes_bootstrap() {
    let directory = TestDirectory::new("lock-bootstrap");
    let store = DurableStore::open_or_create(directory.path()).expect("create durable root");
    let lease = store.acquire_writer().expect("create writer lock");
    drop(lease);
    drop(store);

    let opened = open(directory.path()).expect("resume lock-only bootstrap");
    drop(opened);

    assert_bootstrap_objects(directory.path());
}

#[test]
fn a_marked_partial_bootstrap_with_a_strict_temp_recovers() {
    let directory = TestDirectory::new("partial-bootstrap");
    create_layout_prefix(directory.path(), &["candidates"]);
    write_private(
        &directory.path().join(".pilotage-tmp-17-00000000000000af"),
        b"partial",
    );

    let opened = open(directory.path()).expect("resume partial bootstrap");
    drop(opened);

    assert_bootstrap_objects(directory.path());
    assert!(
        directory
            .path()
            .join(".pilotage-tmp-17-00000000000000af")
            .is_file()
    );
}

#[test]
fn a_partial_bootstrap_rejects_an_out_of_order_directory() {
    let directory = TestDirectory::new("out-of-order-bootstrap");
    create_layout_prefix(directory.path(), &["stages"]);
    let before = catalog(directory.path());

    let error = open(directory.path())
        .err()
        .expect("reject out-of-order bootstrap");

    assert_invalid_layout(error);
    assert_eq!(catalog(directory.path()), before);
    assert!(!directory.path().join("candidates").exists());
    assert!(!directory.path().join("entries").exists());
}

#[test]
fn an_authorized_layout_accepts_a_strict_root_temporary() {
    let directory = TestDirectory::new("authorized-orphan-temp");
    create_authorized_layout(directory.path());
    write_private(
        &directory.path().join(".pilotage-tmp-17-00000000000000af"),
        b"orphan",
    );

    let opened = open(directory.path()).expect("accept strict orphan temporary");
    drop(opened);

    assert!(
        directory
            .path()
            .join(".pilotage-tmp-17-00000000000000af")
            .is_file()
    );
}

#[test]
fn an_acquisition_race_cannot_publish_the_layout_marker() {
    let directory = TestDirectory::new("acquisition-race");
    let store = DurableStore::open_or_create(directory.path()).expect("create durable root");
    let foreign = directory.path().join("legacy-sentinel");

    let error =
        super::layout::open_with_acquisition_hook_for_test(directory.path(), &store, || {
            write_private(&foreign, b"foreign")
        })
        .err()
        .expect("reject a raced foreign object");

    assert_invalid_layout(error);
    assert!(foreign.is_file());
    assert!(directory.path().join(".pilotage-writer-lock").is_file());
    assert!(!directory.path().join(LAYOUT_MARKER).exists());
    for name in ["candidates", "stages", "entries"] {
        assert!(!directory.path().join(name).exists());
    }
}

#[test]
fn an_authorized_incomplete_layout_is_rejected_without_a_change() {
    let directory = TestDirectory::new("incomplete-authorized-layout");
    let store = DurableStore::open_or_create(directory.path()).expect("create durable root");
    let root = store.root_directory();
    let lease = store.acquire_writer().expect("create writer lock");
    root.child(&lease, &name(LAYOUT_MARKER))
        .expect("create layout marker");
    root.child(&lease, &name("candidates"))
        .expect("create candidate directory");
    root.child(&lease, &name("stages"))
        .expect("create stage directory");
    lease
        .compare_exchange_file(
            &root,
            &name("HEAD.json"),
            ExpectedValue::Absent,
            exact_head(Digest::from_bytes([31; 32])).expect("encode head"),
        )
        .expect("publish head");
    drop(lease);
    drop(store);
    let before = catalog(directory.path());

    let error = open(directory.path())
        .err()
        .expect("reject incomplete authorized layout");

    assert_invalid_layout(error);
    assert_eq!(catalog(directory.path()), before);
    assert!(!directory.path().join("entries").exists());
}

#[test]
fn a_marker_does_not_authorize_an_unknown_root_object() {
    let directory = TestDirectory::new("marked-unknown-object");
    create_layout_prefix(directory.path(), &[]);
    write_private(&directory.path().join("legacy-sentinel"), b"not-owned");
    let before = catalog(directory.path());

    let error = open(directory.path())
        .err()
        .expect("reject unknown root object");

    assert_invalid_layout(error);
    assert_eq!(catalog(directory.path()), before);
}

#[test]
fn a_nonempty_marker_cannot_create_missing_data_directories() {
    let directory = TestDirectory::new("nonempty-marker");
    create_layout_prefix(directory.path(), &[]);
    fs::create_dir(directory.path().join(LAYOUT_MARKER).join("foreign"))
        .expect("add marker content");
    let before = catalog(directory.path());

    let error = open(directory.path())
        .err()
        .expect("reject nonempty marker");

    assert_invalid_layout(error);
    assert_eq!(catalog(directory.path()), before);
    for name in ["candidates", "stages", "entries"] {
        assert!(!directory.path().join(name).exists());
    }
}

fn create_layout_prefix(root_path: &Path, directories: &[&str]) {
    let store = DurableStore::open_or_create(root_path).expect("create durable root");
    let root = store.root_directory();
    let lease = store.acquire_writer().expect("create writer lock");
    root.child(&lease, &name(LAYOUT_MARKER))
        .expect("create layout marker");
    for directory in directories {
        root.child(&lease, &name(directory))
            .expect("create partial directory");
    }
}

fn create_authorized_layout(root_path: &Path) {
    let store = DurableStore::open_or_create(root_path).expect("create durable root");
    let root = store.root_directory();
    let lease = store.acquire_writer().expect("create writer lock");
    root.child(&lease, &name(LAYOUT_MARKER))
        .expect("create layout marker");
    for directory in ["candidates", "stages", "entries"] {
        root.child(&lease, &name(directory))
            .expect("create data directory");
    }
    lease
        .compare_exchange_file(
            &root,
            &name("HEAD.json"),
            ExpectedValue::Absent,
            exact_head(Digest::from_bytes([31; 32])).expect("encode head"),
        )
        .expect("publish head");
}

fn assert_bootstrap_objects(root: &Path) {
    for name in [
        ".pilotage-writer-lock",
        LAYOUT_MARKER,
        "candidates",
        "stages",
        "entries",
    ] {
        assert!(root.join(name).exists(), "missing bootstrap object {name}");
    }
}

fn assert_invalid_layout(error: TuneError) {
    assert!(matches!(
        error,
        TuneError::InvalidJournal { detail } if detail.starts_with("journal layout is not valid:")
    ));
}

fn name(value: &str) -> ObjectName {
    ObjectName::new(value).expect("valid object name")
}

fn write_private(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create private file");
    file.write_all(bytes).expect("write private file");
    file.sync_all().expect("sync private file");
}

#[derive(Debug, PartialEq, Eq)]
struct CatalogEntry {
    kind: CatalogKind,
    mode: u32,
    link_count: u64,
    bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogKind {
    Directory,
    File,
    Symlink,
    Other,
}

fn catalog(root: &Path) -> BTreeMap<PathBuf, CatalogEntry> {
    let mut entries = BTreeMap::new();
    collect_catalog(root, root, &mut entries);
    entries
}

fn collect_catalog(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, CatalogEntry>) {
    let metadata = fs::symlink_metadata(path).expect("inspect catalog object");
    let relative = path.strip_prefix(root).expect("relative catalog path");
    let bytes = metadata
        .is_file()
        .then(|| fs::read(path).expect("read catalog object"));
    entries.insert(
        relative.to_path_buf(),
        CatalogEntry {
            kind: catalog_kind(&metadata),
            mode: metadata.permissions().mode() & 0o777,
            link_count: metadata.nlink(),
            bytes,
        },
    );
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .expect("read catalog directory")
            .map(|entry| entry.expect("read catalog entry").path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            collect_catalog(root, &child, entries);
        }
    }
}

fn catalog_kind(metadata: &fs::Metadata) -> CatalogKind {
    let kind = metadata.file_type();
    if kind.is_dir() {
        CatalogKind::Directory
    } else if kind.is_file() {
        CatalogKind::File
    } else if kind.is_symlink() {
        CatalogKind::Symlink
    } else {
        CatalogKind::Other
    }
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = test_root().join(format!("flight-tune-{label}-{}-{time}", std::process::id()));
        Self { path }
    }

    fn with_root(label: &str) -> Self {
        let directory = Self::new(label);
        fs::create_dir(directory.path()).expect("create private root");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("set private root mode");
        directory
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn test_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/private/tmp")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::temp_dir()
            .canonicalize()
            .expect("canonical test temporary directory")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).ok();
    }
}

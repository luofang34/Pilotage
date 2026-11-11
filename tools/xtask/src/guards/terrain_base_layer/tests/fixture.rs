//! A copy of the files the terrain guard reads, in a temporary tree, with
//! edits that must change what they edit.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::super::json::file_digest;
use super::super::{
    APP, BUILDER_LIB, BUILDER_MANIFEST, BUILDER_TESTS, COASTLINE_ARCHIVE, COASTLINE_FETCHER,
    COASTLINE_MANIFEST, COASTLINE_PLAN, COASTLINE_PROVENANCE, FETCHER, GENERATOR, GITIGNORE,
    MANIFEST, MAP_VIEW, PLAN, PROJECT, PROVENANCE, RESOLVER, STYLE, check,
};

pub(super) const EXAMPLE: &str =
    "crates/pilotage-terrain-build/examples/build_situation_fixture.rs";

/// Every file the guard reads, and the example its package scan reads.
const COPIED: [&str; 19] = [
    BUILDER_MANIFEST,
    BUILDER_LIB,
    BUILDER_TESTS,
    EXAMPLE,
    RESOLVER,
    APP,
    STYLE,
    COASTLINE_PLAN,
    COASTLINE_MANIFEST,
    COASTLINE_PROVENANCE,
    PLAN,
    MANIFEST,
    PROVENANCE,
    FETCHER,
    GENERATOR,
    COASTLINE_FETCHER,
    GITIGNORE,
    MAP_VIEW,
    PROJECT,
];

pub(super) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

/// A temporary tree removed when the fixture is dropped.
pub(super) struct Fixture {
    pub(super) root: PathBuf,
}

impl Fixture {
    pub(super) fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("plt_xt_terrain_{tag}_{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let fixture = Self { root };
        for relative in COPIED {
            fixture.restore(relative);
        }
        fixture
    }

    pub(super) fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Puts back the committed copy of one file.
    pub(super) fn restore(&self, relative: &str) {
        let target = self.path(relative);
        std::fs::create_dir_all(target.parent().expect("a parent")).expect("directories");
        std::fs::copy(repo_root().join(relative), target).expect("copy");
    }

    pub(super) fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path(relative)).expect("read")
    }

    pub(super) fn write(&self, relative: &str, text: &str) {
        std::fs::write(self.path(relative), text).expect("write");
    }

    /// Replaces text in a file. An edit that changes nothing proves
    /// nothing, so it fails the test.
    pub(super) fn replace(&self, relative: &str, from: &str, to: &str) {
        let before = self.read(relative);
        let after = before.replacen(from, to, 1);
        assert_ne!(
            before, after,
            "the edit changed nothing in {relative}: {from}"
        );
        self.write(relative, &after);
    }

    /// Removes every line that contains `needle`.
    pub(super) fn drop_lines(&self, relative: &str, needle: &str) {
        let before = self.read(relative);
        let after: String = before
            .split_inclusive('\n')
            .filter(|line| !line.contains(needle))
            .collect();
        assert_ne!(
            before, after,
            "the edit changed nothing in {relative}: {needle}"
        );
        self.write(relative, &after);
    }

    pub(super) fn append(&self, relative: &str, text: &str) {
        let before = self.read(relative);
        self.write(relative, &format!("{before}{text}"));
    }

    /// Edits a JSON file through its value.
    pub(super) fn edit_json(&self, relative: &str, edit: impl FnOnce(&mut Value)) {
        let mut value: Value = serde_json::from_str(&self.read(relative)).expect("JSON");
        let before = value.clone();
        edit(&mut value);
        assert_ne!(before, value, "the edit changed nothing in {relative}");
        self.write(
            relative,
            &serde_json::to_string_pretty(&value).expect("encode"),
        );
    }

    /// A plan edit would fail the manifest digest first, so the manifest
    /// takes the edited plan's digest and fields, and only the invariant
    /// under test stays wrong.
    pub(super) fn resync_coastline_manifest(&self) {
        let plan: Value = serde_json::from_str(&self.read(COASTLINE_PLAN)).expect("plan");
        let digest = file_digest(&self.path(COASTLINE_PLAN));
        let mut manifest: Value =
            serde_json::from_str(&self.read(COASTLINE_MANIFEST)).expect("manifest");
        manifest["plan_sha256"] = Value::String(digest);
        for key in ["dataset_scale", "closest_zoom", "sources", "bands"] {
            manifest[key] = plan[key].clone();
        }
        self.write(
            COASTLINE_MANIFEST,
            &serde_json::to_string_pretty(&manifest).expect("encode"),
        );
    }

    /// A synthetic coastline archive with `tiles` tiles at zoom 7, and a
    /// manifest that records its digest.
    pub(super) fn coastline_archive(&self, tiles: u32) {
        let path = self.path(COASTLINE_ARCHIVE);
        std::fs::remove_file(&path).ok();
        let db = rusqlite::Connection::open(&path).expect("archive");
        db.execute_batch(
            "CREATE TABLE metadata (name text, value text);
             CREATE TABLE tiles (zoom_level integer, tile_column integer,
                 tile_row integer, tile_data blob);
             INSERT INTO metadata VALUES ('json',
                 '{\"vector_layers\":[{\"id\":\"ocean\"},{\"id\":\"land\"},{\"id\":\"lakes\"},{\"id\":\"rivers\"}]}');",
        )
        .expect("schema");
        for column in 0..tiles {
            db.execute("INSERT INTO tiles VALUES (7, ?1, 0, x'00');", [column])
                .expect("tile");
        }
        drop(db);
        let digest = file_digest(&path);
        let mut manifest: Value =
            serde_json::from_str(&self.read(COASTLINE_MANIFEST)).expect("manifest");
        manifest["archive_sha256"] = Value::String(digest);
        self.write(
            COASTLINE_MANIFEST,
            &serde_json::to_string_pretty(&manifest).expect("encode"),
        );
    }

    pub(super) fn refusals(&self) -> Vec<String> {
        check(&self.root)
    }

    /// The guard refuses the tree, and one refusal names `expected`.
    pub(super) fn assert_refused(&self, expected: &str) {
        let refusals = self.refusals();
        assert!(
            refusals.iter().any(|refusal| refusal.contains(expected)),
            "expected a refusal naming {expected:?}, got {refusals:#?}"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

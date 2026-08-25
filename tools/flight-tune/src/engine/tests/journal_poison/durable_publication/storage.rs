use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use super::super::TestDirectory;
use crate::{Digest, JournalEntry};

#[derive(Debug)]
pub(super) struct PublicationSnapshot {
    pub(super) head: Vec<u8>,
    entries: BTreeSet<OsString>,
    candidates: BTreeSet<OsString>,
    stages: BTreeSet<OsString>,
    temporary_count: usize,
}

impl PublicationSnapshot {
    pub(super) fn new(directory: &TestDirectory) -> Self {
        Self {
            head: fs::read(directory.path().join("HEAD.json")).expect("read journal head"),
            entries: names(directory.path().join("entries")),
            candidates: names(directory.path().join("candidates")),
            stages: names(directory.path().join("stages")),
            temporary_count: temporary_count(directory.path()),
        }
    }
}

pub(super) fn assert_publication_objects(
    directory: &TestDirectory,
    before: &PublicationSnapshot,
    after: &PublicationSnapshot,
    candidate: Option<Digest>,
) -> JournalEntry {
    assert_eq!(after.entries.len(), before.entries.len() + 1);
    assert_eq!(
        after.candidates.len(),
        before.candidates.len() + usize::from(candidate.is_some())
    );
    assert_eq!(after.stages, before.stages);
    assert_eq!(before.temporary_count, 0);
    assert_eq!(after.temporary_count, 0);
    let entry_name = single_new_name(&before.entries, &after.entries);
    let entry = read_content_addressed(
        directory.path().join("entries").join(&entry_name),
        &entry_name,
    );
    if let Some(expected) = candidate {
        let candidate_name = single_new_name(&before.candidates, &after.candidates);
        let digest = content_addressed_digest(
            directory.path().join("candidates").join(&candidate_name),
            &candidate_name,
        );
        assert_eq!(digest, expected);
    }
    entry
}

pub(super) fn assert_head_matches(directory: &TestDirectory, entry: &JournalEntry) {
    let expected =
        crate::identity::digest_bytes(&serde_json::to_vec(entry).expect("encode journal entry"));
    let head: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.path().join("HEAD.json")).expect("read journal head"),
    )
    .expect("decode journal head");
    assert_eq!(
        head.get("digest"),
        Some(&serde_json::to_value(expected).expect("encode digest"))
    );
}

fn read_content_addressed(path: impl AsRef<Path>, name: &OsString) -> JournalEntry {
    let bytes = fs::read(path).expect("read new journal entry");
    assert_content_addressed_name(name, &bytes);
    serde_json::from_slice(&bytes).expect("decode new journal entry")
}

fn content_addressed_digest(path: impl AsRef<Path>, name: &OsString) -> crate::Digest {
    let bytes = fs::read(path).expect("read new candidate");
    assert_content_addressed_name(name, &bytes)
}

fn assert_content_addressed_name(name: &OsString, bytes: &[u8]) -> crate::Digest {
    let digest = crate::identity::digest_bytes(bytes);
    assert_eq!(name, &OsString::from(format!("{digest}.json")));
    digest
}

fn single_new_name(before: &BTreeSet<OsString>, after: &BTreeSet<OsString>) -> OsString {
    let names = after.difference(before).cloned().collect::<Vec<_>>();
    assert_eq!(names.len(), 1);
    names[0].clone()
}

fn names(path: impl AsRef<Path>) -> BTreeSet<OsString> {
    fs::read_dir(path)
        .expect("read object directory")
        .map(|entry| entry.expect("read object entry").file_name())
        .collect()
}

fn temporary_count(path: &Path) -> usize {
    fs::read_dir(path)
        .expect("read durable directory")
        .map(|entry| entry.expect("read durable entry"))
        .map(|entry| {
            let own = usize::from(
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(".pilotage-tmp-")),
            );
            own + temporary_count_if_directory(&entry.path())
        })
        .sum()
}

fn temporary_count_if_directory(path: &Path) -> usize {
    if path.is_dir() {
        temporary_count(path)
    } else {
        0
    }
}

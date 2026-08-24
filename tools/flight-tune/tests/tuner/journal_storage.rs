use std::fs;

use flight_tune::TuneError;
use pilotage_durable_storage::StorageError;

use super::test_rig::{FakeHandle, SequenceStrategy};
use super::{TestDirectory, open};

#[test]
fn a_semantic_but_noncanonical_head_is_rejected() {
    let directory = TestDirectory::new("noncanonical-head");
    let state = FakeHandle::new();
    let tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");
    drop(tuner);

    let head_path = directory.path().join("HEAD.json");
    let canonical = fs::read(&head_path).expect("read canonical head");
    let mut noncanonical = vec![b' '];
    noncanonical.extend(canonical);
    fs::write(&head_path, noncanonical).expect("write noncanonical head");

    let result = open(
        directory.path(),
        state,
        SequenceStrategy::new(Vec::new()),
        2.0,
    );
    assert!(matches!(
        result,
        Err(TuneError::InvalidJournal { detail })
            if detail == "the journal head does not use canonical bytes"
    ));
}

#[test]
fn changed_candidate_bytes_fail_the_digest_named_read() {
    let directory = TestDirectory::new("candidate-content-mismatch");
    let state = FakeHandle::new();
    let tuner = open(
        directory.path(),
        state.clone(),
        SequenceStrategy::new(Vec::new()),
        2.0,
    )
    .expect("open tuner");
    drop(tuner);

    let candidate_path = fs::read_dir(directory.path().join("candidates"))
        .expect("read candidate directory")
        .next()
        .expect("candidate object")
        .expect("read candidate entry")
        .path();
    let canonical = fs::read(&candidate_path).expect("read candidate object");
    let mut changed = vec![b' '];
    changed.extend(canonical);
    fs::write(candidate_path, changed).expect("change candidate object");

    let result = open(
        directory.path(),
        state,
        SequenceStrategy::new(Vec::new()),
        2.0,
    );
    assert!(matches!(
        result,
        Err(TuneError::Storage { source })
            if matches!(source.as_ref(), StorageError::ContentMismatch { context }
                if context.object.is_some())
    ));
}

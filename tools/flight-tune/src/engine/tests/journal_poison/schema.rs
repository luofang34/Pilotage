use std::fs;

use pilotage_durable_storage::FaultController;

use super::rig::{
    EnvelopeGates, FakeBackend, FakeFactory, FakeHandle, QuadraticMetric, SequenceStrategy,
    candidate, stage,
};
use super::{TestDirectory, TestTuner};
use crate::identity::digest_bytes;
use crate::{Digest, JournalEntry, TuneError, Tuner};

#[test]
fn schema_four_active_campaign_stops_before_all_external_action() {
    let directory = TestDirectory::new("schema-four-no-external-action");
    create_schema_five_campaign(&directory);
    rewrite_started_entry_as_schema_four(&directory);
    let state = FakeHandle::new();

    let error = open_existing_campaign(&directory, state.clone())
        .err()
        .expect("reject schema four campaign");

    assert!(matches!(error, TuneError::InvalidJournal { .. }));
    assert_no_external_action(&state);
}

fn create_schema_five_campaign(directory: &TestDirectory) {
    let state = FakeHandle::new();
    let tuner = open_existing_campaign(directory, state).expect("create schema five campaign");
    assert_eq!(tuner.journal().entries().len(), 1);
}

fn open_existing_campaign(
    directory: &TestDirectory,
    state: FakeHandle,
) -> Result<TestTuner, TuneError> {
    Tuner::open_or_resume_with_faults(
        directory.path(),
        stage(),
        91,
        candidate(0.0),
        FakeBackend::new(state.clone()),
        FakeFactory::new(state.clone()),
        EnvelopeGates::tracked(2.0, state.clone()),
        QuadraticMetric::new(state),
        SequenceStrategy::new(Vec::new()),
        FaultController::default(),
    )
}

fn rewrite_started_entry_as_schema_four(directory: &TestDirectory) {
    let head_path = directory.path().join("HEAD.json");
    let head: HeadPointer =
        serde_json::from_slice(&fs::read(&head_path).expect("read HEAD")).expect("decode HEAD");
    let old_path = entry_path(directory, head.digest);
    let mut entry: JournalEntry =
        serde_json::from_slice(&fs::read(&old_path).expect("read started entry"))
            .expect("decode started entry");
    entry.schema_version = 4;
    let bytes = serde_json::to_vec(&entry).expect("encode schema four entry");
    let digest = digest_bytes(&bytes);
    let new_path = entry_path(directory, digest);
    fs::rename(old_path, &new_path).expect("rename schema four entry");
    fs::write(new_path, bytes).expect("write schema four entry");
    fs::write(
        head_path,
        serde_json::to_vec(&HeadPointer { digest }).expect("encode schema four HEAD"),
    )
    .expect("write schema four HEAD");
}

fn entry_path(directory: &TestDirectory, digest: Digest) -> std::path::PathBuf {
    directory
        .path()
        .join("entries")
        .join(format!("{digest}.json"))
}

fn assert_no_external_action(state: &FakeHandle) {
    let state = state.0.borrow();
    assert!(state.lifecycle.is_empty());
    assert_eq!(state.open_session_count, 0);
    assert_eq!(state.vehicle.bind_count, 0);
    assert_eq!(state.terminal.bind_count(), 0);
    assert_eq!(state.terminal.causal_evidence_read_count(), 0);
    assert_eq!(state.terminal.recover_count(), 0);
    assert_eq!(state.terminal.seal_count(), 0);
    assert_eq!(state.stop_count, 0);
    assert_eq!(state.cleanup_count, 0);
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HeadPointer {
    digest: Digest,
}

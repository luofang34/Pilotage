use std::collections::HashSet;
use std::path::{Path, PathBuf};

use pilotage_durable_storage::{
    ContentDigest, DurableDirectory, DurableStore, ExactObject, ExpectedValue, ObjectName,
    PutOutcome, StorageError, WriterLease,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

mod append;
mod layout;

pub(super) use append::append_entry;
pub(super) use append::append_entry_with_hook;

use crate::journal::JournalEntry;
use crate::{Candidate, Digest, SearchStage, TuneError};

#[cfg(test)]
#[path = "storage/tests.rs"]
mod tests;

const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHAIN_ENTRIES: usize = 100_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadPointer {
    digest: Digest,
}

pub(super) struct JournalStorage {
    _store: DurableStore,
    root: DurableDirectory,
    marker: DurableDirectory,
    candidates: DurableDirectory,
    stages: DurableDirectory,
    entries: DurableDirectory,
}

pub(super) type WriterLock = WriterLease;

pub(super) fn open(root: &Path) -> Result<(JournalStorage, WriterLock), TuneError> {
    let store = DurableStore::open_or_create(root).map_err(storage_error)?;
    finish_open(root, store)
}

#[cfg(test)]
pub(super) fn open_with_faults(
    root: &Path,
    faults: pilotage_durable_storage::FaultController,
) -> Result<(JournalStorage, WriterLock), TuneError> {
    let store = DurableStore::open_or_create_with_faults(root, faults).map_err(storage_error)?;
    finish_open(root, store)
}

fn finish_open(
    root_path: &Path,
    store: DurableStore,
) -> Result<(JournalStorage, WriterLock), TuneError> {
    let opened = layout::open(root_path, &store)?;
    Ok((
        JournalStorage {
            _store: store,
            root: opened.root,
            marker: opened.marker,
            candidates: opened.candidates,
            stages: opened.stages,
            entries: opened.entries,
        },
        opened.writer,
    ))
}

pub(super) fn head_exists(storage: &JournalStorage) -> Result<bool, TuneError> {
    storage
        .root
        .exists(&object_name("HEAD.json")?)
        .map_err(storage_error)
}

pub(super) fn verify_head_exact(
    storage: &JournalStorage,
    expected: Digest,
) -> Result<(), TuneError> {
    let name = object_name("HEAD.json")?;
    let expected_head = exact_head(expected)?;
    let actual = storage
        .root
        .read_digest(&name, expected_head.digest(), MAX_DOCUMENT_BYTES)
        .map_err(storage_error)?;
    if actual != expected_head {
        return Err(invalid_journal(
            "the journal head does not match the live authorization",
        ));
    }
    Ok(())
}

pub(super) fn verify_live_snapshot(
    storage: &JournalStorage,
    writer: &WriterLock,
    stage: &SearchStage,
    entries: &[JournalEntry],
    entry_digests: &[Digest],
) -> Result<(), TuneError> {
    verify_live_snapshot_with_hook(storage, writer, stage, entries, entry_digests, || {})
}

#[cfg(test)]
pub(super) fn verify_live_snapshot_with_final_hook_for_test(
    storage: &JournalStorage,
    writer: &WriterLock,
    stage: &SearchStage,
    entries: &[JournalEntry],
    entry_digests: &[Digest],
    before_final_writer_validation: impl FnOnce(),
) -> Result<(), TuneError> {
    verify_live_snapshot_with_hook(
        storage,
        writer,
        stage,
        entries,
        entry_digests,
        before_final_writer_validation,
    )
}

/// Verifies that this process still holds authority over the exact live head.
///
/// The check reads the layout marker, the four directory handles, the writer
/// lease, and the head pointer. It reads no chain entry, no search stage, and
/// no candidate, so its cost is the same at every journal length.
pub(super) fn verify_live_authority(
    storage: &JournalStorage,
    writer: &WriterLock,
    entry_digests: &[Digest],
) -> Result<(), TuneError> {
    layout::verify_authorized(&storage.root)?;
    layout::verify_handles(
        &storage.marker,
        &storage.candidates,
        &storage.stages,
        &storage.entries,
    )?;
    writer.validate(&storage.root).map_err(storage_error)?;
    verify_head_exact(storage, live_head(entry_digests)?)
}

fn live_head(entry_digests: &[Digest]) -> Result<Digest, TuneError> {
    entry_digests
        .last()
        .copied()
        .ok_or_else(|| invalid_journal("the live journal has no head"))
}

fn verify_live_snapshot_with_hook(
    storage: &JournalStorage,
    writer: &WriterLock,
    stage: &SearchStage,
    entries: &[JournalEntry],
    entry_digests: &[Digest],
    before_final_writer_validation: impl FnOnce(),
) -> Result<(), TuneError> {
    verify_live_authority(storage, writer, entry_digests)?;
    verify_entry_chain(storage, entries, entry_digests)?;
    verify_stage_and_candidates(storage, stage, entries)?;
    verify_head_exact(storage, live_head(entry_digests)?)?;
    layout::verify_authorized(&storage.root)?;
    layout::verify_handles(
        &storage.marker,
        &storage.candidates,
        &storage.stages,
        &storage.entries,
    )?;
    before_final_writer_validation();
    writer.validate(&storage.root).map_err(storage_error)
}

fn verify_entry_chain(
    storage: &JournalStorage,
    entries: &[JournalEntry],
    entry_digests: &[Digest],
) -> Result<(), TuneError> {
    let stored = load_entries(storage)?;
    let matches = entries.len() == entry_digests.len()
        && stored.len() == entries.len()
        && stored.iter().zip(entry_digests.iter().zip(entries)).all(
            |((stored_digest, stored_entry), (live_digest, live_entry))| {
                stored_digest == live_digest && stored_entry == live_entry
            },
        );
    if matches {
        Ok(())
    } else {
        Err(invalid_journal(
            "the durable journal chain does not match the live journal",
        ))
    }
}

fn verify_stage_and_candidates(
    storage: &JournalStorage,
    stage: &SearchStage,
    entries: &[JournalEntry],
) -> Result<(), TuneError> {
    let session = entries
        .first()
        .map(|entry| &entry.session)
        .ok_or_else(|| invalid_journal("the live journal has no session"))?;
    if read_stage(storage, session.stage_digest)? != *stage {
        return Err(invalid_journal(
            "the durable search stage does not match the live stage",
        ));
    }
    let initial = read_candidate(storage, session.initial_candidate_digest)?;
    for candidate in candidate_digests_to_verify(session.initial_candidate_digest, entries) {
        let stored = read_candidate(storage, candidate)?;
        stage.validate_challenger(&initial, &stored)?;
    }
    Ok(())
}

fn candidate_digests_to_verify(initial: Digest, entries: &[JournalEntry]) -> Vec<Digest> {
    let mut seen = HashSet::from([initial]);
    entries
        .iter()
        .filter_map(|entry| match entry.event {
            crate::JournalEvent::CandidateTransitionAuthorized { candidate, .. }
            | crate::JournalEvent::AttemptPrepared { candidate, .. } => Some(candidate),
            _ => None,
        })
        .filter(|candidate| seen.insert(*candidate))
        .collect()
}

pub(super) fn document_digest<T: Serialize>(
    document: &'static str,
    value: &T,
) -> Result<Digest, TuneError> {
    Ok(exact_document(document, value)?.0)
}

pub(super) fn store_candidate(
    storage: &JournalStorage,
    writer: &WriterLock,
    candidate: &Candidate,
) -> Result<Digest, TuneError> {
    let (digest, object) = exact_document("candidate", candidate)?;
    write_immutable(
        &storage.candidates,
        writer,
        &object_name(format!("{digest}.json"))?,
        &object,
        digest,
    )?;
    Ok(digest)
}

pub(super) fn read_candidate(
    storage: &JournalStorage,
    digest: Digest,
) -> Result<Candidate, TuneError> {
    let name = object_name(format!("{digest}.json"))?;
    let bytes = read_verified(&storage.candidates, &name, digest)?;
    let candidate: Candidate = decode("candidate", &name, &bytes)?;
    candidate.validate()?;
    Ok(candidate)
}

pub(super) fn store_stage(
    storage: &JournalStorage,
    writer: &WriterLock,
    stage: &SearchStage,
) -> Result<Digest, TuneError> {
    let (digest, object) = exact_document("search stage", stage)?;
    write_immutable(
        &storage.stages,
        writer,
        &object_name(format!("{digest}.json"))?,
        &object,
        digest,
    )?;
    Ok(digest)
}

pub(super) fn read_stage(
    storage: &JournalStorage,
    digest: Digest,
) -> Result<SearchStage, TuneError> {
    let name = object_name(format!("{digest}.json"))?;
    let bytes = read_verified(&storage.stages, &name, digest)?;
    let stage: SearchStage = decode("search stage", &name, &bytes)?;
    stage.validate()?;
    Ok(stage)
}

pub(super) fn load_entries(
    storage: &JournalStorage,
) -> Result<Vec<(Digest, JournalEntry)>, TuneError> {
    let head_name = object_name("HEAD.json")?;
    let head_object = storage
        .root
        .read_exact(&head_name, MAX_DOCUMENT_BYTES)
        .map_err(storage_error)?;
    let head: HeadPointer = decode("journal head", &head_name, head_object.bytes())?;
    if head_object.bytes() != exact_head(head.digest)?.bytes() {
        return Err(invalid_journal(
            "the journal head does not use canonical bytes",
        ));
    }
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(head.digest);
    while let Some(digest) = next {
        if entries.len() >= MAX_CHAIN_ENTRIES || !seen.insert(digest) {
            return Err(invalid_journal(
                "the journal chain is too long or has a cycle",
            ));
        }
        let name = object_name(format!("{digest}.json"))?;
        let bytes = read_verified(&storage.entries, &name, digest)?;
        let entry: JournalEntry = decode("journal entry", &name, &bytes)?;
        next = entry.previous;
        entries.push((digest, entry));
    }
    entries.reverse();
    Ok(entries)
}

fn expected_head(previous: Option<Digest>) -> Result<ExpectedValue, TuneError> {
    previous
        .map(exact_head)
        .transpose()
        .map(|head| head.map_or(ExpectedValue::Absent, ExpectedValue::Exact))
}

fn exact_head(digest: Digest) -> Result<ExactObject, TuneError> {
    let bytes = encode("journal head", &HeadPointer { digest })?;
    Ok(ExactObject::from_bytes(bytes))
}

fn exact_document<T: Serialize>(
    document: &'static str,
    value: &T,
) -> Result<(Digest, ExactObject), TuneError> {
    let object = ExactObject::from_bytes(encode(document, value)?);
    let digest = digest_from_storage(object.digest());
    Ok((digest, object))
}

fn write_immutable(
    directory: &DurableDirectory,
    writer: &WriterLock,
    name: &ObjectName,
    object: &ExactObject,
    digest: Digest,
) -> Result<(), TuneError> {
    if digest_from_storage(object.digest()) != digest {
        return Err(TuneError::DigestMismatch { expected: digest });
    }
    let outcome = directory
        .put_immutable_no_replace(writer, name, object)
        .map_err(storage_error)?;
    match outcome {
        PutOutcome::Published | PutOutcome::AlreadyExact => Ok(()),
    }
}

fn read_verified(
    directory: &DurableDirectory,
    name: &ObjectName,
    expected: Digest,
) -> Result<Vec<u8>, TuneError> {
    let expected_storage = ContentDigest(*expected.as_bytes());
    let object = directory
        .read_digest(name, expected_storage, MAX_DOCUMENT_BYTES)
        .map_err(storage_error)?;
    if digest_from_storage(object.digest()) != expected {
        return Err(TuneError::DigestMismatch { expected });
    }
    Ok(object.bytes().to_vec())
}

fn encode<T: Serialize>(document: &'static str, value: &T) -> Result<Vec<u8>, TuneError> {
    let bytes =
        serde_json::to_vec(value).map_err(|source| TuneError::Encode { document, source })?;
    check_size(document.to_owned(), bytes.len())?;
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(
    document: &'static str,
    name: &ObjectName,
    bytes: &[u8],
) -> Result<T, TuneError> {
    serde_json::from_slice(bytes).map_err(|source| TuneError::Decode {
        document,
        path: PathBuf::from(name.as_os_str()),
        source,
    })
}

fn check_size(document: String, size: usize) -> Result<(), TuneError> {
    if size > MAX_DOCUMENT_BYTES {
        return Err(TuneError::DocumentTooLarge {
            document,
            size: u64::try_from(size).unwrap_or(u64::MAX),
            limit: u64::try_from(MAX_DOCUMENT_BYTES).unwrap_or(u64::MAX),
        });
    }
    Ok(())
}

fn object_name(name: impl AsRef<std::ffi::OsStr>) -> Result<ObjectName, TuneError> {
    ObjectName::new(name).map_err(storage_error)
}

fn digest_from_storage(digest: ContentDigest) -> Digest {
    Digest::from_bytes(digest.0)
}

fn writer_error(path: &Path, source: StorageError) -> TuneError {
    if source.is_writer_locked() {
        TuneError::JournalLocked {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    } else {
        storage_error(source)
    }
}

fn storage_error(source: StorageError) -> TuneError {
    TuneError::Storage {
        source: Box::new(source),
    }
}

fn invalid_journal(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}

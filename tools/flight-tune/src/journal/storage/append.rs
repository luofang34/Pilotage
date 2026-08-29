use pilotage_durable_storage::{CasOutcome, CompareExchangeError};

use super::{
    JournalStorage, WriterLock, decode, document_digest, exact_document, exact_head, expected_head,
    head_exists, invalid_journal, layout, object_name, read_candidate, read_stage, read_verified,
    storage_error, verify_head_exact, verify_live_snapshot, verify_stage_and_candidates,
    write_immutable,
};
use crate::journal::{JournalEntry, JournalEvent};
use crate::{Candidate, Digest, SearchStage, TuneError};

pub(in crate::journal) fn append_entry(
    storage: &JournalStorage,
    writer: &WriterLock,
    stage: &SearchStage,
    entries: &[JournalEntry],
    entry_digests: &[Digest],
) -> Result<Digest, TuneError> {
    append_entry_with_hook(storage, writer, stage, entries, entry_digests, || {})
}

pub(in crate::journal) fn append_entry_with_hook(
    storage: &JournalStorage,
    writer: &WriterLock,
    stage: &SearchStage,
    entries: &[JournalEntry],
    entry_digests: &[Digest],
    before_authorization: impl FnOnce(),
) -> Result<Digest, TuneError> {
    append_entry_inner(
        storage,
        writer,
        stage,
        entries,
        entry_digests,
        before_authorization,
    )
}

fn append_entry_inner(
    storage: &JournalStorage,
    writer: &WriterLock,
    stage: &SearchStage,
    entries: &[JournalEntry],
    entry_digests: &[Digest],
    before_authorization: impl FnOnce(),
) -> Result<Digest, TuneError> {
    let ProspectiveParts {
        entry,
        expected_digest,
        current_entries,
        current_digests,
    } = prospective_parts(entries, entry_digests)?;
    verify_prospective_snapshot(
        storage,
        writer,
        stage,
        entries,
        current_entries,
        current_digests,
        false,
    )?;
    let (digest, object) = exact_document("journal entry", entry)?;
    if digest != expected_digest {
        return Err(TuneError::DigestMismatch {
            expected: expected_digest,
        });
    }
    let entry_name = object_name(format!("{digest}.json"))?;
    write_immutable(&storage.entries, writer, &entry_name, &object, digest)?;
    verify_prospective_snapshot(
        storage,
        writer,
        stage,
        entries,
        current_entries,
        current_digests,
        true,
    )?;

    let expected = expected_head(entry.previous)?;
    let new_head = exact_head(digest)?;
    let outcome = writer
        .compare_exchange_file_guarded(
            &storage.root,
            &object_name("HEAD.json")?,
            expected,
            new_head,
            || {
                before_authorization();
                verify_prospective_snapshot(
                    storage,
                    writer,
                    stage,
                    entries,
                    current_entries,
                    current_digests,
                    true,
                )
            },
        )
        .map_err(map_guarded_error)?;
    match outcome {
        CasOutcome::Exchanged | CasOutcome::AlreadyExact => Ok(digest),
    }
}

fn map_guarded_error(error: CompareExchangeError<TuneError>) -> TuneError {
    match error {
        CompareExchangeError::Storage { source } => storage_error(source),
        CompareExchangeError::Validation { source } => source,
        CompareExchangeError::ValidationAndCleanup {
            validation,
            cleanup,
        } => TuneError::AuthorizationAndCleanupFailed {
            authorization: Box::new(validation),
            cleanup: Box::new(cleanup),
        },
    }
}

struct ProspectiveParts<'a> {
    entry: &'a JournalEntry,
    expected_digest: Digest,
    current_entries: &'a [JournalEntry],
    current_digests: &'a [Digest],
}

fn prospective_parts<'a>(
    entries: &'a [JournalEntry],
    entry_digests: &'a [Digest],
) -> Result<ProspectiveParts<'a>, TuneError> {
    let (entry, current_entries) = entries
        .split_last()
        .ok_or_else(|| invalid_journal("the prospective journal has no entry"))?;
    let (digest, current_digests) = entry_digests
        .split_last()
        .ok_or_else(|| invalid_journal("the prospective journal has no digest"))?;
    if current_entries.len() != current_digests.len() {
        return Err(invalid_journal(
            "the prospective journal digest count does not match",
        ));
    }
    Ok(ProspectiveParts {
        entry,
        expected_digest: *digest,
        current_entries,
        current_digests,
    })
}

#[allow(clippy::too_many_arguments)]
fn verify_prospective_snapshot(
    storage: &JournalStorage,
    writer: &WriterLock,
    stage: &SearchStage,
    entries: &[JournalEntry],
    current_entries: &[JournalEntry],
    current_digests: &[Digest],
    entry_must_exist: bool,
) -> Result<(), TuneError> {
    if current_entries.is_empty() {
        verify_initial_snapshot(storage, writer)?;
    } else {
        verify_live_snapshot(storage, writer, stage, current_entries, current_digests)?;
    }
    verify_stage_and_candidates(storage, stage, entries)?;
    let entry = entries
        .last()
        .ok_or_else(|| invalid_journal("the prospective journal has no entry"))?;
    verify_prospective_entry_references(storage, entry)?;
    if entry_must_exist {
        verify_prospective_entry_object(storage, entry)?;
    }
    verify_current_authorization_tail(storage, writer, current_digests)
}

fn verify_initial_snapshot(storage: &JournalStorage, writer: &WriterLock) -> Result<(), TuneError> {
    layout::verify_initial_authorization(&storage.root)?;
    layout::verify_handles(
        &storage.marker,
        &storage.candidates,
        &storage.stages,
        &storage.entries,
    )?;
    if head_exists(storage)? {
        return Err(invalid_journal("the initial journal already has a head"));
    }
    writer.validate(&storage.root).map_err(storage_error)
}

fn verify_prospective_entry_object(
    storage: &JournalStorage,
    entry: &JournalEntry,
) -> Result<(), TuneError> {
    let expected = document_digest("journal entry", entry)?;
    let name = object_name(format!("{expected}.json"))?;
    let bytes = read_verified(&storage.entries, &name, expected)?;
    let stored: JournalEntry = decode("journal entry", &name, &bytes)?;
    if stored == *entry {
        Ok(())
    } else {
        Err(invalid_journal(
            "the prospective journal entry bytes do not match",
        ))
    }
}

fn verify_current_authorization_tail(
    storage: &JournalStorage,
    writer: &WriterLock,
    current_digests: &[Digest],
) -> Result<(), TuneError> {
    if let Some(digest) = current_digests.last().copied() {
        verify_head_exact(storage, digest)?;
        layout::verify_authorized(&storage.root)?;
    } else {
        if head_exists(storage)? {
            return Err(invalid_journal("the initial journal already has a head"));
        }
        layout::verify_initial_authorization(&storage.root)?;
    }
    layout::verify_handles(
        &storage.marker,
        &storage.candidates,
        &storage.stages,
        &storage.entries,
    )?;
    writer.validate(&storage.root).map_err(storage_error)
}

fn verify_prospective_entry_references(
    storage: &JournalStorage,
    entry: &JournalEntry,
) -> Result<(), TuneError> {
    let stage = read_stage(storage, entry.session.stage_digest)?;
    let initial = read_candidate(storage, entry.session.initial_candidate_digest)?;
    stage.validate_challenger(&initial, &initial)?;
    match &entry.event {
        JournalEvent::Started { candidate }
        | JournalEvent::CandidateTransitionAuthorized { candidate, .. }
        | JournalEvent::AttemptPrepared { candidate, .. }
        | JournalEvent::Sealed { candidate, .. } => {
            verify_candidate_reference(storage, &stage, &initial, *candidate)
        }
        JournalEvent::Frozen {
            baseline,
            candidate,
        } => {
            verify_candidate_reference(storage, &stage, &initial, *baseline)?;
            verify_candidate_reference(storage, &stage, &initial, *candidate)
        }
        JournalEvent::AttemptCompleted { .. }
        | JournalEvent::RunPrepared { .. }
        | JournalEvent::RunBound { .. }
        | JournalEvent::RunTerminalIntentPrepared { .. }
        | JournalEvent::RunTerminalReportRecorded { .. }
        | JournalEvent::RunTerminalEvidenceFailureRecorded { .. }
        | JournalEvent::RunCommitted { .. }
        | JournalEvent::AttemptQuarantined { .. }
        | JournalEvent::RetryAuthorized { .. }
        | JournalEvent::RetryExhausted { .. }
        | JournalEvent::CleanupRecorded { .. }
        | JournalEvent::PromotionClosed { .. } => Ok(()),
    }
}

fn verify_candidate_reference(
    storage: &JournalStorage,
    stage: &SearchStage,
    initial: &Candidate,
    digest: Digest,
) -> Result<(), TuneError> {
    let candidate = read_candidate(storage, digest)?;
    stage.validate_challenger(initial, &candidate)
}

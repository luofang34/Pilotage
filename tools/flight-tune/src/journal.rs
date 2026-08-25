//! Content-addressed tuning journal and campaign state.

mod attempt;
mod event;
mod replay;
mod storage;
mod transition;

pub use event::{
    AttemptRole, CampaignPhase, FinalQualificationOutcome, JournalEvent, OperationStatus,
    PromotionDecision,
};

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::identity::harness_build_identity;
use crate::{
    Candidate, CandidateLineage, Digest, RuntimeIdentities, SearchStage, TrainingObservation,
    TuneError,
};
use replay::{JournalState, replay};

const JOURNAL_SCHEMA_VERSION: u32 = 3;

/// The immutable identity of one tuning session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionIdentity {
    /// The digest of the complete search stage.
    pub stage_digest: Digest,
    /// The digest of the initial released candidate.
    pub initial_candidate_digest: Digest,
    /// The source identity for every candidate.
    pub candidate_lineage: CandidateLineage,
    /// The fixed seed for all planned runs and proposals.
    pub fixed_seed: u64,
    /// All executable and plant identities.
    pub runtimes: RuntimeIdentities,
}

/// One content-addressed record in the tuning journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEntry {
    /// The journal schema version.
    pub schema_version: u32,
    /// The sequence number in this journal.
    pub sequence: u64,
    /// The prior journal record digest.
    pub previous: Option<Digest>,
    /// The immutable tuning session identity.
    pub session: SessionIdentity,
    /// The saved tuning event.
    pub event: JournalEvent,
}

/// A content-addressed journal with one locked writer and one atomic head.
pub struct Journal {
    storage: storage::JournalStorage,
    stage: SearchStage,
    writer: storage::WriterLock,
    poisoned: AtomicBool,
    entries: Vec<JournalEntry>,
    entry_digests: Vec<Digest>,
    state: JournalState,
}

impl Journal {
    /// Opens a matching journal or starts a new journal.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when identity, storage, or replay is not valid.
    pub fn open_or_create(
        root: impl AsRef<Path>,
        stage: &SearchStage,
        fixed_seed: u64,
        runtimes: RuntimeIdentities,
        initial_candidate: &Candidate,
    ) -> Result<Self, TuneError> {
        Self::validate_open_inputs(stage, &runtimes, initial_candidate)?;
        let opened = storage::open(root.as_ref())?;
        Self::open_with_storage(opened, stage, fixed_seed, runtimes, initial_candidate)
    }

    #[cfg(test)]
    pub(crate) fn open_or_create_with_faults(
        root: impl AsRef<Path>,
        stage: &SearchStage,
        fixed_seed: u64,
        runtimes: RuntimeIdentities,
        initial_candidate: &Candidate,
        faults: pilotage_durable_storage::FaultController,
    ) -> Result<Self, TuneError> {
        Self::validate_open_inputs(stage, &runtimes, initial_candidate)?;
        let opened = storage::open_with_faults(root.as_ref(), faults)?;
        Self::open_with_storage(opened, stage, fixed_seed, runtimes, initial_candidate)
    }

    fn validate_open_inputs(
        stage: &SearchStage,
        runtimes: &RuntimeIdentities,
        initial_candidate: &Candidate,
    ) -> Result<(), TuneError> {
        stage.validate()?;
        initial_candidate.validate()?;
        stage.validate_challenger(initial_candidate, initial_candidate)?;
        runtimes.validate()?;
        if runtimes.harness_build != harness_build_identity() {
            return Err(TuneError::InvalidIdentity {
                detail: "the harness build identity does not match this build".to_owned(),
            });
        }
        Ok(())
    }

    fn open_with_storage(
        opened: (storage::JournalStorage, storage::WriterLock),
        stage: &SearchStage,
        fixed_seed: u64,
        runtimes: RuntimeIdentities,
        initial_candidate: &Candidate,
    ) -> Result<Self, TuneError> {
        let (storage, writer) = opened;
        let candidate_digest = storage::document_digest("candidate", initial_candidate)?;
        let session = SessionIdentity {
            stage_digest: storage::document_digest("search stage", stage)?,
            initial_candidate_digest: candidate_digest,
            candidate_lineage: initial_candidate.lineage().clone(),
            fixed_seed,
            runtimes,
        };
        if storage::head_exists(&storage)? {
            return Self::resume(storage, stage.clone(), writer, session);
        }
        Self::start(storage, stage.clone(), writer, session, initial_candidate)
    }

    /// Returns the immutable session identity.
    #[must_use]
    pub fn session(&self) -> &SessionIdentity {
        &self.entries[0].session
    }

    /// Returns the digest of the immutable session identity.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when JSON encoding fails.
    pub fn session_digest(&self) -> Result<Digest, TuneError> {
        storage::document_digest("session identity", self.session())
    }

    /// Returns all journal entries in sequence order.
    #[must_use]
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// Returns the current campaign phase.
    #[must_use]
    pub const fn phase(&self) -> CampaignPhase {
        self.state.phase
    }

    /// Returns the completed training challenger count.
    #[must_use]
    pub const fn training_attempt_count(&self) -> u64 {
        self.state.training_attempt_count
    }

    /// Reads the current training incumbent.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the candidate artifact is not valid.
    pub fn training_incumbent(&self) -> Result<Candidate, TuneError> {
        self.ensure_usable()?;
        self.read_candidate(self.state.training_incumbent)
    }

    pub(crate) fn read_candidate(&self, digest: Digest) -> Result<Candidate, TuneError> {
        self.ensure_usable()?;
        storage::read_candidate(&self.storage, digest).inspect_err(|_| {
            self.poisoned.store(true, Ordering::Release);
        })
    }

    pub(crate) fn ensure_usable(&self) -> Result<(), TuneError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(TuneError::JournalPoisoned);
        }
        storage::verify_live_snapshot(
            &self.storage,
            &self.writer,
            &self.stage,
            &self.entries,
            &self.entry_digests,
        )
        .inspect_err(|_| {
            self.poisoned.store(true, Ordering::Release);
        })
    }

    #[cfg(test)]
    pub(crate) fn ensure_usable_with_final_hook_for_test(
        &self,
        before_final_writer_validation: impl FnOnce(),
    ) -> Result<(), TuneError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(TuneError::JournalPoisoned);
        }
        storage::verify_live_snapshot_with_final_hook_for_test(
            &self.storage,
            &self.writer,
            &self.stage,
            &self.entries,
            &self.entry_digests,
            before_final_writer_validation,
        )
        .inspect_err(|_| {
            self.poisoned.store(true, Ordering::Release);
        })
    }

    fn record_storage_result<T>(&self, result: Result<T, TuneError>) -> Result<T, TuneError> {
        result.inspect_err(|error| {
            if error.poisons_journal() {
                self.poisoned.store(true, Ordering::Release);
            }
        })
    }

    fn record_append_result<T>(&self, result: Result<T, TuneError>) -> Result<T, TuneError> {
        result.inspect_err(|_| {
            self.poisoned.store(true, Ordering::Release);
        })
    }

    pub(crate) fn state(&self) -> &JournalState {
        &self.state
    }

    pub(crate) fn training_history(&self) -> Vec<TrainingObservation> {
        self.state.training_history.clone()
    }

    #[cfg(test)]
    pub(crate) fn append_event_with_before_authorization_for_test(
        &mut self,
        event: JournalEvent,
        before_authorization: impl FnOnce(),
    ) -> Result<(), TuneError> {
        self.append_with_hook(event, before_authorization)
    }

    pub(crate) fn freeze(&mut self) -> Result<Digest, TuneError> {
        self.ensure_usable()?;
        let candidate = self.state.training_incumbent;
        self.append(JournalEvent::Frozen {
            baseline: self.session().initial_candidate_digest,
            candidate,
        })?;
        Ok(candidate)
    }

    pub(crate) fn close_promotion(&mut self, decision: PromotionDecision) -> Result<(), TuneError> {
        self.append(JournalEvent::PromotionClosed { decision })
    }

    pub(crate) fn seal(
        &mut self,
        candidate: Digest,
        outcome: FinalQualificationOutcome,
    ) -> Result<(), TuneError> {
        self.append(JournalEvent::Sealed { candidate, outcome })
    }

    fn start(
        storage: storage::JournalStorage,
        stage: SearchStage,
        writer: storage::WriterLock,
        session: SessionIdentity,
        initial_candidate: &Candidate,
    ) -> Result<Self, TuneError> {
        let stored = storage::store_stage(&storage, &writer, &stage)?;
        if stored != session.stage_digest {
            return Err(TuneError::DigestMismatch { expected: stored });
        }
        let candidate = storage::store_candidate(&storage, &writer, initial_candidate)?;
        let entry = JournalEntry {
            schema_version: JOURNAL_SCHEMA_VERSION,
            sequence: 0,
            previous: None,
            session,
            event: JournalEvent::Started { candidate },
        };
        let digest = storage::document_digest("journal entry", &entry)?;
        let entries = vec![entry];
        let entry_digests = vec![digest];
        let state = replay(&entries, &entry_digests, &stage)?;
        let published = storage::append_entry(&storage, &writer, &stage, &entries, &entry_digests)?;
        if published != digest {
            return Err(TuneError::DigestMismatch { expected: digest });
        }
        Ok(Self {
            storage,
            stage,
            writer,
            poisoned: AtomicBool::new(false),
            entries,
            entry_digests,
            state,
        })
    }

    fn resume(
        storage: storage::JournalStorage,
        stage: SearchStage,
        writer: storage::WriterLock,
        session: SessionIdentity,
    ) -> Result<Self, TuneError> {
        let stored_entries = storage::load_entries(&storage)?;
        let (entry_digests, entries): (Vec<_>, Vec<_>) = stored_entries.into_iter().unzip();
        if entries.first().map(|entry| &entry.session) != Some(&session)
            || storage::read_stage(&storage, session.stage_digest)? != stage
        {
            return Err(TuneError::JournalSessionMismatch);
        }
        let state = replay(&entries, &entry_digests, &stage)?;
        let journal = Self {
            storage,
            stage,
            writer,
            poisoned: AtomicBool::new(false),
            entries,
            entry_digests,
            state,
        };
        journal.ensure_usable()?;
        Ok(journal)
    }

    fn append(&mut self, event: JournalEvent) -> Result<(), TuneError> {
        self.append_with_hook(event, || {})
    }

    fn append_with_hook(
        &mut self,
        event: JournalEvent,
        before_authorization: impl FnOnce(),
    ) -> Result<(), TuneError> {
        self.ensure_usable()?;
        let previous = self
            .entries
            .last()
            .ok_or_else(|| invalid("journal has no head"))?;
        let entry = JournalEntry {
            schema_version: JOURNAL_SCHEMA_VERSION,
            sequence: previous.sequence.wrapping_add(1),
            previous: self.entry_digests.last().copied(),
            session: previous.session.clone(),
            event,
        };
        let mut entries = self.entries.clone();
        entries.push(entry.clone());
        let entry_digest = storage::document_digest("journal entry", &entry)?;
        let mut digests = self.entry_digests.clone();
        digests.push(entry_digest);
        let state = replay(&entries, &digests, &self.stage)?;
        let published = storage::append_entry_with_hook(
            &self.storage,
            &self.writer,
            &self.stage,
            &entries,
            &digests,
            before_authorization,
        );
        if self.record_append_result(published)? != entry_digest {
            return Err(TuneError::DigestMismatch {
                expected: entry_digest,
            });
        }
        self.entries = entries;
        self.entry_digests = digests;
        self.state = state;
        Ok(())
    }
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}

//! Content-addressed tuning journal and campaign state.

mod event;
mod replay;
mod storage;

pub use event::{
    AttemptRole, CampaignPhase, FinalQualificationOutcome, JournalEvent, OperationStatus,
    PromotionDecision,
};

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::identity::harness_build_identity;
use crate::{
    Candidate, CandidateEvaluation, CandidateLineage, Digest, RuntimeIdentities, SearchStage,
    TrainingObservation, TuneError,
};
use replay::{JournalState, replay};

const JOURNAL_SCHEMA_VERSION: u32 = 2;

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
    root: PathBuf,
    stage: SearchStage,
    _writer_lock: storage::WriterLock,
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
        stage.validate()?;
        initial_candidate.validate()?;
        stage.validate_challenger(initial_candidate, initial_candidate)?;
        runtimes.validate()?;
        if runtimes.harness_build != harness_build_identity() {
            return Err(TuneError::InvalidIdentity {
                detail: "the harness build identity does not match this build".to_owned(),
            });
        }
        let root = root.as_ref().to_path_buf();
        storage::ensure_layout(&root)?;
        let writer_lock = storage::acquire_writer_lock(&root)?;
        let candidate_digest = storage::document_digest("candidate", initial_candidate)?;
        let session = SessionIdentity {
            stage_digest: storage::document_digest("search stage", stage)?,
            initial_candidate_digest: candidate_digest,
            candidate_lineage: initial_candidate.lineage().clone(),
            fixed_seed,
            runtimes,
        };
        if storage::head_exists(&root)? {
            return Self::resume(root, stage.clone(), writer_lock, session);
        }
        Self::start(root, stage.clone(), writer_lock, session, initial_candidate)
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
        self.read_candidate(self.state.training_incumbent)
    }

    pub(crate) fn read_candidate(&self, digest: Digest) -> Result<Candidate, TuneError> {
        storage::read_candidate(&self.root, digest)
    }

    pub(crate) fn state(&self) -> &JournalState {
        &self.state
    }

    pub(crate) fn training_history(&self) -> Vec<TrainingObservation> {
        self.state.training_history.clone()
    }

    pub(crate) fn prepare_attempt(
        &mut self,
        role: AttemptRole,
        candidate: &Candidate,
        plan_digest: Digest,
    ) -> Result<(u64, Digest), TuneError> {
        candidate.validate()?;
        let candidate_digest = storage::store_candidate(&self.root, candidate)?;
        let trial_id = self.state.next_trial_id;
        self.append(JournalEvent::AttemptPrepared {
            trial_id,
            role,
            candidate: candidate_digest,
            plan_digest,
        })?;
        Ok((trial_id, candidate_digest))
    }

    pub(crate) fn complete_attempt(
        &mut self,
        trial_id: u64,
        evaluation: CandidateEvaluation,
        selected: Option<bool>,
    ) -> Result<(), TuneError> {
        let role = self.state.pending_role(trial_id)?;
        evaluation.validate(role.scenario_set())?;
        self.append(JournalEvent::AttemptCompleted {
            trial_id,
            evaluation,
            selected_as_training_incumbent: selected,
        })
    }

    pub(crate) fn quarantine_attempt(
        &mut self,
        trial_id: u64,
        reason: impl Into<String>,
    ) -> Result<(), TuneError> {
        self.append(JournalEvent::AttemptQuarantined {
            trial_id,
            reason: reason.into(),
        })
    }

    pub(crate) fn record_cleanup(
        &mut self,
        trial_id: u64,
        stop: OperationStatus,
        cleanup: OperationStatus,
    ) -> Result<(), TuneError> {
        self.append(JournalEvent::CleanupRecorded {
            trial_id,
            stop,
            cleanup,
        })
    }

    pub(crate) fn freeze(&mut self) -> Result<Digest, TuneError> {
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
        root: PathBuf,
        stage: SearchStage,
        writer_lock: storage::WriterLock,
        session: SessionIdentity,
        initial_candidate: &Candidate,
    ) -> Result<Self, TuneError> {
        let stored = storage::store_stage(&root, &stage)?;
        if stored != session.stage_digest {
            return Err(TuneError::DigestMismatch { expected: stored });
        }
        let candidate = storage::store_candidate(&root, initial_candidate)?;
        let entry = JournalEntry {
            schema_version: JOURNAL_SCHEMA_VERSION,
            sequence: 0,
            previous: None,
            session,
            event: JournalEvent::Started { candidate },
        };
        let digest = storage::append_entry(&root, &entry)?;
        let entries = vec![entry];
        let entry_digests = vec![digest];
        let state = replay(&entries, &entry_digests, &stage)?;
        Ok(Self {
            root,
            stage,
            _writer_lock: writer_lock,
            entries,
            entry_digests,
            state,
        })
    }

    fn resume(
        root: PathBuf,
        stage: SearchStage,
        writer_lock: storage::WriterLock,
        session: SessionIdentity,
    ) -> Result<Self, TuneError> {
        let stored_entries = storage::load_entries(&root)?;
        let (entry_digests, entries): (Vec<_>, Vec<_>) = stored_entries.into_iter().unzip();
        if entries.first().map(|entry| &entry.session) != Some(&session)
            || storage::read_stage(&root, session.stage_digest)? != stage
        {
            return Err(TuneError::JournalSessionMismatch);
        }
        let state = replay(&entries, &entry_digests, &stage)?;
        let journal = Self {
            root,
            stage,
            _writer_lock: writer_lock,
            entries,
            entry_digests,
            state,
        };
        journal.verify_candidates()?;
        Ok(journal)
    }

    fn append(&mut self, event: JournalEvent) -> Result<(), TuneError> {
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
        if storage::append_entry(&self.root, &entry)? != entry_digest {
            return Err(TuneError::DigestMismatch {
                expected: entry_digest,
            });
        }
        self.entries = entries;
        self.entry_digests = digests;
        self.state = state;
        Ok(())
    }

    fn verify_candidates(&self) -> Result<(), TuneError> {
        let initial = self.read_candidate(self.session().initial_candidate_digest)?;
        for entry in &self.entries {
            if let JournalEvent::AttemptPrepared { candidate, .. } = entry.event {
                let stored = self.read_candidate(candidate)?;
                self.stage.validate_challenger(&initial, &stored)?;
            }
        }
        Ok(())
    }
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}

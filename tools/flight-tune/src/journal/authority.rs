//! The two live checks a journal holder makes before it acts.
//!
//! A holder proves two different things about the same journal.
//!
//! *Authority* is the right to act at all: this process still holds the writer
//! lease over an authorized layout, and the durable head is still the exact
//! entry this holder believes it wrote. Its cost is the same at every journal
//! length.
//!
//! *The catalog audit* is the stronger claim that every durable byte this
//! holder has ever verified is still that byte: the complete entry chain, the
//! stored search stage, and every candidate the chain names. Its cost grows
//! with the journal, so the interval it holds at is a deliberate choice rather
//! than a consequence of where the check is convenient to call.
//!
//! `Journal::ensure_usable` makes both claims and states that interval: it
//! runs at every campaign action boundary and inside every durable append, so
//! an in-place change to an object this holder already verified refuses the
//! next action and can never enter the chain. `Journal::ensure_authority`
//! makes only the first claim, for the sample stream, where no entry is
//! appended and the audit would otherwise repeat once for every telemetry
//! sample.

use std::sync::atomic::Ordering;

use super::{Journal, storage};
use crate::TuneError;

impl Journal {
    /// Verifies the live authority and audits the complete durable catalog.
    ///
    /// This is the check every campaign action boundary makes. It refuses an
    /// in-place change to any durable object the journal names, including one
    /// no live operation reads.
    pub(crate) fn ensure_usable(&self) -> Result<(), TuneError> {
        self.guard(|| {
            storage::verify_live_snapshot(
                &self.storage,
                &self.writer,
                &self.stage,
                &self.entries,
                &self.entry_digests,
            )
        })
    }

    /// Verifies the live authority alone, without the catalog audit.
    ///
    /// The sample stream calls this before each external action on the
    /// vehicle. It refuses a moved head, a changed layout, and a lost writer
    /// lease, which are the conditions that take away the right to act. It
    /// does not read the chain, so a change to an already-verified object is
    /// refused at the run terminal instead of at the sample that follows it.
    pub(crate) fn ensure_authority(&self) -> Result<(), TuneError> {
        self.guard(|| {
            storage::verify_live_authority(&self.storage, &self.writer, &self.entry_digests)
        })
    }

    pub(crate) fn poison(&self) {
        self.poisoned.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn ensure_usable_with_final_hook_for_test(
        &self,
        before_final_writer_validation: impl FnOnce(),
    ) -> Result<(), TuneError> {
        self.guard(|| {
            storage::verify_live_snapshot_with_final_hook_for_test(
                &self.storage,
                &self.writer,
                &self.stage,
                &self.entries,
                &self.entry_digests,
                before_final_writer_validation,
            )
        })
    }

    pub(super) fn record_storage_result<T>(
        &self,
        result: Result<T, TuneError>,
    ) -> Result<T, TuneError> {
        result.inspect_err(|error| {
            if error.poisons_journal() {
                self.poison();
            }
        })
    }

    pub(super) fn record_append_result<T>(
        &self,
        result: Result<T, TuneError>,
    ) -> Result<T, TuneError> {
        result.inspect_err(|_| self.poison())
    }

    fn guard(&self, verify: impl FnOnce() -> Result<(), TuneError>) -> Result<(), TuneError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(TuneError::JournalPoisoned);
        }
        verify().inspect_err(|_| self.poison())
    }
}

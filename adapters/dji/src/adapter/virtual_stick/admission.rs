use std::time::{Duration, Instant};

use super::{DjiVirtualStickDiagnosticError as Error, DjiVirtualStickDiagnosticStatus};
use crate::adapter::{DjiAdapter, DjiControlMode, input::Framing};

impl DjiAdapter {
    /// Queue one bounded enable-and-disable diagnostic without velocity writes.
    ///
    /// The diagnostic opt-in must be set at startup. The caller must continue
    /// to call `step` until the retained status ends. The absolute deadline
    /// applies to the enable write; cleanup has a fixed additional budget.
    /// Dropping the adapter cannot prove that the FC disabled the mode.
    ///
    /// # Errors
    /// Returns an error if the experiment is disabled, evidence is missing,
    /// another command is pending, or this attachment already had a diagnostic.
    pub fn request_virtual_stick_diagnostic(
        &mut self,
        request_id: u64,
        send_before: Instant,
    ) -> Result<(), Error> {
        let now = self.origin.elapsed();
        let until = send_before
            .checked_duration_since(self.origin)
            .ok_or(Error::InvalidDeadline)?;
        self.request_virtual_stick_at(request_id, now, until)
    }

    /// Request the diagnostic's single bounded cleanup attempt.
    ///
    /// This does not start I/O. Keep stepping to attempt the disable request.
    /// It does not stop motors or prove that control returned to the RC.
    pub fn cancel_virtual_stick_diagnostic(&mut self) {
        if !self.connection.mode_session.retained() {
            self.connection.mode_session.cancel();
        }
    }

    /// Read retained diagnostic evidence without renewing any source age.
    #[must_use]
    pub fn virtual_stick_diagnostic_status_cached(
        &self,
    ) -> Option<DjiVirtualStickDiagnosticStatus> {
        (!self.connection.mode_session.retained())
            .then(|| self.connection.mode_session.status_cached())
            .flatten()
    }

    pub(in crate::adapter) fn request_virtual_stick_at(
        &mut self,
        request_id: u64,
        now: Duration,
        send_until: Duration,
    ) -> Result<(), Error> {
        if !self.experimental_virtual_stick_diagnostic
            || self.control.mode() != DjiControlMode::Observed
            || self.connection.input.framing() != Framing::LogicLink
        {
            return Err(Error::Disabled);
        }
        self.connection.queue_mode_session(
            request_id,
            self.control.sequence,
            now,
            send_until,
            &self.observations,
        )?;
        self.control.cancel_observed();
        self.control.sequence = self.control.sequence.wrapping_add(2);
        Ok(())
    }
}

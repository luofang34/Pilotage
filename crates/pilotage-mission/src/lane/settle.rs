//! Pictures, their judgments and the evidence windows (ADR-0052). A verdict
//! goes to the run that issued its ticket: the run in progress, or a run a
//! successor replaced, whose tasks still take the answers due inside their
//! windows. The windows run on the host's monotonic clock, which does not
//! stop while the run holds, so a held lane still closes them. At most
//! [`MAX_SETTLING_RUNS`] replaced runs wait for answers; past that the
//! oldest closes at once.

use super::MAX_SETTLING_RUNS;
use super::evidence::{FrameTicket, VerdictRefused};
use super::run::{FrameAssessment, RunTracker};
use crate::lane::Lane;

impl Lane {
    /// Takes the host's monotonic time, milliseconds since the host started,
    /// and closes the evidence windows that ran out. A host process that
    /// restarts builds a new lane; it never compares times of two processes.
    pub fn on_host_clock(&mut self, now_ms: u64) {
        self.host_ms = self.host_ms.max(now_ms);
        let (now_ms, window_ms) = (self.host_ms, self.config.evidence_window_ms);
        let mut changed = false;
        for run in self.past.iter_mut().chain(self.run.as_mut()) {
            changed |= run.seal(now_ms, window_ms, false);
        }
        self.changed |= changed;
    }

    /// Keeps `previous`, which a successor replaced, for the answers still
    /// due to it; the oldest replaced runs close when too many wait.
    pub(super) fn retire(&mut self, mut previous: RunTracker) {
        previous.supersede(self.host_ms);
        self.past.push(previous);
        let now_ms = self.host_ms;
        let mut settling = self.past.iter().filter(|r| r.settling()).count();
        for run in &mut self.past {
            if settling <= MAX_SETTLING_RUNS {
                break;
            }
            if run.settling() {
                run.seal(now_ms, 0, true);
                settling = settling.saturating_sub(1);
            }
        }
    }

    /// The run that issued `ticket`.
    fn issuer(&mut self, ticket: &FrameTicket) -> Option<&mut RunTracker> {
        self.run
            .iter_mut()
            .chain(self.past.iter_mut().rev())
            .find(|run| run.issued(ticket))
    }

    /// Counts a captured picture against the active survey line. Returns
    /// its ticket, for a judgment, when a line is active.
    pub fn on_frame(&mut self) -> Option<FrameTicket> {
        if !self.is_flying() {
            return None;
        }
        let ticket = self.run.as_mut()?.on_frame()?;
        self.changed = true;
        Some(ticket)
    }

    /// Whether the line `ticket` names still needs pictures sent for
    /// judgment to meet its evidence: the host sends such a picture without
    /// waiting for its sampling period, so a short line is judged too.
    #[must_use]
    pub fn wants_judgment(&self, ticket: &FrameTicket) -> bool {
        self.run
            .as_ref()
            .is_some_and(|run| run.wants_judgment(ticket))
    }

    /// Notes the picture `ticket` names as sent for judgment: the line's
    /// evidence is pending until its verdict or the news that it was not
    /// judged.
    pub fn on_sent(&mut self, ticket: &FrameTicket) {
        if let Some(run) = self.issuer(ticket)
            && run.on_sent(ticket)
        {
            self.changed = true;
        }
    }

    /// Records the judgment of the picture `ticket` names; `None` when the
    /// service did not judge it.
    ///
    /// # Errors
    ///
    /// [`VerdictRefused`] for a verdict of a run the lane does not know, of
    /// a picture never sent, or a second verdict of one picture; the lane
    /// changes nothing. A verdict after its task's evidence window closed is
    /// refused as late and only counted.
    pub fn on_assessment(
        &mut self,
        ticket: &FrameTicket,
        assessment: Option<FrameAssessment>,
    ) -> Result<(), VerdictRefused> {
        let run = self
            .issuer(ticket)
            .ok_or_else(|| VerdictRefused::OtherRun {
                run_id: ticket.run_id.clone(),
                task_id: ticket.task_id.clone(),
            })?;
        let result = run.on_assessment(ticket, assessment);
        if matches!(result, Ok(true) | Err(VerdictRefused::Late { .. })) {
            self.changed = true;
        }
        result.map(|_| ())
    }
}

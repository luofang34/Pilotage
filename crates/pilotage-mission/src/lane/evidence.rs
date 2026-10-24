//! A judgment's identity and a task's completion evidence. Each picture a
//! lane sends for judgment carries a ticket that names its run, its task,
//! the task's attempt and the picture, so a verdict for another run, for a
//! picture never sent, or a second verdict for one picture changes nothing.
//! A task's evidence is pending while a sent picture has no answer.

use std::collections::BTreeSet;

use pilotage_protocol::wire;

/// The identity of one picture sent for judgment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FrameTicket {
    /// The run the picture was captured in.
    pub run_id: String,
    /// The task it was captured on.
    pub task_id: String,
    /// The task's attempt in that run, from one.
    pub attempt: u32,
    /// The picture's number in the task's attempt, from one.
    pub frame: u32,
}

/// Why a lane does not take a verdict.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerdictRefused {
    /// The verdict is for a run that is not the lane's run.
    #[error("the verdict for {task_id} is for run {run_id}, not the lane's run")]
    OtherRun {
        /// The verdict's run.
        run_id: String,
        /// The verdict's task.
        task_id: String,
    },
    /// The verdict names a task or an attempt the run does not have.
    #[error("the verdict names {task_id} attempt {attempt}, which the run does not fly")]
    UnknownTask {
        /// The verdict's task.
        task_id: String,
        /// The verdict's attempt.
        attempt: u32,
    },
    /// The picture was never sent for judgment.
    #[error("picture {frame} of {task_id} was not sent for judgment")]
    NotSent {
        /// The verdict's task.
        task_id: String,
        /// The verdict's picture.
        frame: u32,
    },
    /// The picture already has a verdict.
    #[error("picture {frame} of {task_id} already has a verdict")]
    Duplicate {
        /// The verdict's task.
        task_id: String,
        /// The verdict's picture.
        frame: u32,
    },
}

/// One task's pictures sent for judgment and the ones answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Judgments {
    sent: BTreeSet<u32>,
    answered: BTreeSet<u32>,
}

impl Judgments {
    pub(super) fn sent(&mut self, frame: u32) {
        self.sent.insert(frame);
    }

    /// Takes the answer for `frame`, refusing a picture never sent or one
    /// already answered.
    pub(super) fn answer(&mut self, ticket: &FrameTicket) -> Result<(), VerdictRefused> {
        if !self.sent.contains(&ticket.frame) {
            return Err(VerdictRefused::NotSent {
                task_id: ticket.task_id.clone(),
                frame: ticket.frame,
            });
        }
        if !self.answered.insert(ticket.frame) {
            return Err(VerdictRefused::Duplicate {
                task_id: ticket.task_id.clone(),
                frame: ticket.frame,
            });
        }
        Ok(())
    }

    /// Where the evidence stands, for a lane that has a judgment service
    /// when `judged`.
    pub(super) fn state(
        &self,
        evidence: &wire::MissionTaskEvidence,
        judged: bool,
        min_usable: u32,
    ) -> wire::MissionEvidenceState {
        if !judged {
            wire::MissionEvidenceState::NotJudged
        } else if self.answered.len() < self.sent.len() {
            wire::MissionEvidenceState::Pending
        } else if evidence.assessed == 0 {
            wire::MissionEvidenceState::NotJudged
        } else if evidence.usable >= min_usable {
            wire::MissionEvidenceState::Met
        } else {
            wire::MissionEvidenceState::Missing
        }
    }
}

//! A scripted survey agent: an automated client that steers a lane only
//! through the commands every client sends. It proposes the survey, asks
//! to approve its own proposal (a host with an approver secret refuses
//! it), waits for a person to approve, starts the run, and resumes a hold
//! when the host lets it. A hold a person set it cannot undo, so it waits
//! for that person to resume.

use pilotage_client_session::mission::{ApprovalPolicy, MissionCommand, SurveyFile};
use pilotage_protocol::wire::MissionLaneState as Lane;
use tokio::time::Instant;

use super::{Session, terminal};
use crate::cli::OperatorError;

impl Session {
    pub(super) async fn survey_agent(
        &mut self,
        objective: SurveyFile,
        deadline: Instant,
    ) -> Result<bool, OperatorError> {
        self.output.note("proposing the survey");
        let proposed = self.ask(&MissionCommand::Propose { objective }).await?;
        if !proposed.is_some_and(|r| r.accepted()) {
            self.output.note("the lane refused the proposal; stopping");
            return Ok(false);
        }
        self.output
            .note("asking to approve my own proposal; only an approver may");
        let own = MissionCommand::Approve {
            digest: None,
            policy: ApprovalPolicy::default(),
        };
        if self.ask(&own).await?.is_some_and(|r| r.accepted()) {
            self.output
                .note("the host let me approve: it has no approver secret");
        } else {
            self.output.note("waiting for a person to approve");
        }
        while self.desk.state() != Lane::Admitted {
            if let Some(ended) = terminal(self.desk.state()) {
                return Ok(ended);
            }
            self.step(deadline).await?;
        }
        self.output.note("approved; starting the run");
        if !self
            .ask(&MissionCommand::Start { digest: None })
            .await?
            .is_some_and(|r| r.accepted())
        {
            self.output.note("the lane refused the start; stopping");
            return Ok(false);
        }
        self.follow_run(deadline).await
    }

    /// Follows the run to its end, asking once to resume each hold.
    async fn follow_run(&mut self, deadline: Instant) -> Result<bool, OperatorError> {
        let mut asked_for_this_hold = false;
        loop {
            match self.desk.state() {
                Lane::Held | Lane::HoldRequested if !asked_for_this_hold => {
                    asked_for_this_hold = true;
                    self.output.note("the run is held; asking to resume it");
                    let resumed = self.ask(&MissionCommand::Resume {}).await?;
                    if !resumed.is_some_and(|r| r.accepted()) {
                        self.output
                            .note("not mine to resume; waiting for the person who held it");
                    }
                }
                Lane::Held | Lane::HoldRequested => {}
                state => {
                    asked_for_this_hold = false;
                    if let Some(ended) = terminal(state) {
                        self.output.note(if ended {
                            "the survey is complete"
                        } else {
                            "the run ended without completing"
                        });
                        return Ok(ended);
                    }
                }
            }
            self.step(deadline).await?;
        }
    }
}

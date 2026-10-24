//! An instruction's commands, sent one at a time: a step goes to its lane only after the
//! lane accepts the step before it, so a step that depends on an earlier one never
//! reaches a lane that refused the earlier one. The first step a lane does not accept
//! stops the rest.
//!
//! The dispatch does no I/O. The client polls it with the desks of its lanes, sends what
//! it returns through the lane's desk, and polls again when a receipt arrives.

use std::collections::VecDeque;

use pilotage_protocol::wire;

use crate::mission::{IntentKind, MissionCommand, MissionDesk};

/// How long a desk may take to send a step before the dispatch stops; a desk that
/// refuses a command sends nothing and says why in its own notice.
const SEND_WAIT_MS: u64 = 5_000;

/// How long a lane may take to answer a step: a team answers after its members do.
const ANSWER_WAIT_MS: u64 = 60_000;

/// What the client does next for a dispatch.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// Send `command` through the desk of `lane` now.
    Send {
        /// The lane.
        lane: String,
        /// The command.
        command: MissionCommand,
    },
    /// The step sent last has no answer yet.
    Waiting,
    /// The lanes accepted every step.
    Done,
    /// A lane did not accept a step, so no later step was sent.
    Stopped {
        /// The lane.
        lane: String,
        /// Why, in words for the person.
        reason: String,
    },
}

/// The steps not sent yet, and the one that waits for its answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Dispatch {
    pending: VecDeque<(String, MissionCommand)>,
    waiting: Option<Waiting>,
    stopped: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Waiting {
    lane: String,
    /// The request id the desk gives the step; another intent sent through the same
    /// desk meanwhile has an id of its own and answers nothing here.
    request: String,
    kind: IntentKind,
    since_ms: u64,
}

impl Dispatch {
    /// A dispatch of `commands`, each with its lane, in order.
    pub fn new(commands: Vec<(String, MissionCommand)>) -> Self {
        Self {
            pending: commands.into(),
            waiting: None,
            stopped: false,
        }
    }

    /// The steps not sent yet.
    pub fn remaining(&self) -> usize {
        self.pending.len()
    }

    /// What to do next at `now_ms`, with `desk` giving the desk of a lane by name.
    pub fn poll<'d>(
        &mut self,
        desk: impl Fn(&str) -> Option<&'d MissionDesk>,
        now_ms: u64,
    ) -> Progress {
        if let Some(waiting) = self.waiting.take() {
            match answer(&waiting, desk(&waiting.lane), now_ms) {
                Answer::Accepted => {}
                Answer::Waiting => {
                    self.waiting = Some(waiting);
                    return Progress::Waiting;
                }
                Answer::Stopped(reason) => return self.stop(waiting.lane, reason),
            }
        }
        if self.stopped {
            return Progress::Done;
        }
        let Some((lane, command)) = self.pending.pop_front() else {
            return Progress::Done;
        };
        let Some(request) = desk(&lane).map(MissionDesk::next_request) else {
            return self.stop(lane, "the lane is no longer attached".to_owned());
        };
        self.waiting = Some(Waiting {
            lane: lane.clone(),
            request,
            kind: command.kind(),
            since_ms: now_ms,
        });
        Progress::Send { lane, command }
    }

    fn stop(&mut self, lane: String, reason: String) -> Progress {
        self.pending.clear();
        self.stopped = true;
        Progress::Stopped { lane, reason }
    }
}

enum Answer {
    Accepted,
    Waiting,
    Stopped(String),
}

fn answer(waiting: &Waiting, desk: Option<&MissionDesk>, now_ms: u64) -> Answer {
    let Some(desk) = desk else {
        return Answer::Stopped("the lane is no longer attached".to_owned());
    };
    let waited = now_ms.saturating_sub(waiting.since_ms);
    match desk.sent(&waiting.request) {
        None if waited > SEND_WAIT_MS => {
            return Answer::Stopped(
                "the desk did not send the step; its notice says why".to_owned(),
            );
        }
        None => return Answer::Waiting,
        // The desk refused the step, and the id went to an intent sent another way.
        Some(kind) if kind != waiting.kind => {
            return Answer::Stopped(
                "the desk did not send the step; another command went first".to_owned(),
            );
        }
        Some(_) => {}
    }
    let receipt = desk.receipt_for(&waiting.request);
    match receipt {
        Some(receipt) if receipt.accepted() => Answer::Accepted,
        Some(receipt) => Answer::Stopped(match receipt.outcome {
            wire::MissionReceiptOutcome::Uncertain => {
                format!(
                    "the lane has not seen the step take effect: {}",
                    receipt.reason
                )
            }
            _ => format!("refused: {}", receipt.reason),
        }),
        None if waited > ANSWER_WAIT_MS => Answer::Stopped("the lane did not answer".to_owned()),
        None => Answer::Waiting,
    }
}

//! What the coordinator may send while its handover journal is or is not
//! written (ADR-0052). An intent that gives a member work waits until the
//! journal holds the handovers it acts on: it is prepared, then persisted,
//! then sent, and never sent ahead of its record. A hold or an abort never
//! waits for the journal, as a stop must not depend on a disk. While a write
//! fails, the prepared intents wait in a bounded backlog, the coordinator
//! takes no new work, and it says so.
//!
//! A hold for a member with waiting work goes out at once, and it waits
//! again behind that work: when the disk recovers, the member gets the work
//! and then the hold, in the order a written journal gives. So the disk does
//! not change what a hold does.
//!
//! Work that will never go out is withheld, and the lane is told: an abort
//! for a member withholds that member's waiting work, and an answer whose work does not fit the
//! backlog is withheld whole, so no part of a fan-out goes out without the
//! rest. The lane takes a withheld intent as refused by its member, which is
//! true, as the member never saw it. A restart restores the records only and
//! replays no intent.

use pilotage_coordination::MemberIntent;
use pilotage_protocol::wire;

/// The most prepared intents that wait for the journal.
pub(super) const BACKLOG_LIMIT: usize = 256;

/// The prepared intents and the journal state they wait on.
#[derive(Debug, Default)]
pub(super) struct Dispatch {
    backlog: Vec<MemberIntent>,
    /// The handovers the journal holds.
    journaled: Vec<wire::MissionHandover>,
    /// The last write failed and has not succeeded since.
    failing: bool,
}

/// An intent that never goes out, and why.
#[derive(Debug, PartialEq)]
pub(super) struct Withheld {
    pub intent: MemberIntent,
    pub reason: &'static str,
}

/// What to do after the lane answered.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Step {
    /// Intents that go out now: stops, or work whose record is written.
    pub send: Vec<MemberIntent>,
    /// The handovers to write before the backlog may go out.
    pub write: Option<Vec<wire::MissionHandover>>,
    /// Intents that never go out, for the lane to take as refused.
    pub withheld: Vec<Withheld>,
}

const ABORTED: &str =
    "not sent: an abort reached the member before the coordinator journal let this go out";
const FULL: &str = "not sent: the coordinator journal cannot be written and its backlog is full";

/// Whether `intent` aborts a member's work.
fn is_abort(intent: &MemberIntent) -> bool {
    matches!(
        intent.intent.action,
        Some(wire::mission_intent::Action::Abort(_))
    )
}

/// Whether `intent` stops a member: it never waits for the journal.
pub(super) fn is_stop(intent: &MemberIntent) -> bool {
    matches!(
        intent.intent.action,
        Some(wire::mission_intent::Action::Hold(_) | wire::mission_intent::Action::Abort(_))
    )
}

impl Dispatch {
    /// A dispatcher whose journal holds `journaled`.
    pub(super) fn new(journaled: Vec<wire::MissionHandover>) -> Self {
        Self {
            journaled,
            ..Self::default()
        }
    }

    /// Takes the intents the lane produced, in order, with its handovers
    /// now: stops go out at once, work waits for the journal. While the
    /// journal fails, the write is left to [`Dispatch::retry`].
    pub(super) fn offer(
        &mut self,
        intents: Vec<MemberIntent>,
        handovers: Vec<wire::MissionHandover>,
    ) -> Step {
        let mut step = Step::default();
        let work = intents.iter().filter(|i| !is_stop(i)).count();
        let fits = self.backlog.len().saturating_add(work) <= BACKLOG_LIMIT;
        for intent in intents {
            if is_abort(&intent) {
                let (ended, kept) = std::mem::take(&mut self.backlog)
                    .into_iter()
                    .partition(|waiting| waiting.member == intent.member);
                self.backlog = kept;
                step.withheld.extend(withheld(ended, ABORTED));
                step.send.push(intent);
            } else if is_stop(&intent) {
                if self.backlog.iter().any(|w| w.member == intent.member) {
                    self.backlog.push(intent.clone());
                }
                step.send.push(intent);
            } else if fits {
                self.backlog.push(intent);
            } else {
                step.withheld.push(Withheld {
                    intent,
                    reason: FULL,
                });
            }
        }
        if self.failing {
            return step;
        }
        if handovers == self.journaled {
            step.send.append(&mut self.backlog);
        } else {
            step.write = Some(handovers);
        }
        step
    }

    /// The write to try again while the journal fails.
    pub(super) fn retry(
        &self,
        handovers: Vec<wire::MissionHandover>,
    ) -> Option<Vec<wire::MissionHandover>> {
        self.failing.then_some(handovers)
    }

    /// The journal write of `handovers` succeeded or failed. On success the
    /// backlog goes out, once.
    pub(super) fn written(
        &mut self,
        handovers: Vec<wire::MissionHandover>,
        succeeded: bool,
    ) -> Vec<MemberIntent> {
        if !succeeded {
            self.failing = true;
            return Vec::new();
        }
        self.failing = false;
        self.journaled = handovers;
        std::mem::take(&mut self.backlog)
    }

    /// The prepared intents that wait for the journal.
    #[cfg(test)]
    pub(super) fn waiting(&self) -> usize {
        self.backlog.len()
    }

    /// The journal cannot be written: only stops are taken.
    pub(super) fn blocks_new_work(&self) -> bool {
        self.failing
    }

    /// Why new work is refused, for the operator.
    pub(super) fn refusal(&self) -> String {
        format!(
            "the coordinator cannot write its handover journal; {} intents wait for it; only hold, abort and fetch are taken",
            self.backlog.len()
        )
    }
}

/// The work of `intents`, withheld for `reason`. A hold waiting behind
/// that work is dropped with it: it was sent already.
fn withheld(intents: Vec<MemberIntent>, reason: &'static str) -> Vec<Withheld> {
    intents
        .into_iter()
        .filter(|intent| !is_stop(intent))
        .map(|intent| Withheld { intent, reason })
        .collect()
}

#[cfg(test)]
mod tests;

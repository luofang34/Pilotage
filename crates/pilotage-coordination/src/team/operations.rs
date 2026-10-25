//! The operator requests a team answers only once its members answered
//! (ADR-0052). Each is an operation: the operator's request, the member
//! requests sent for it, and what came of each. One operation of a kind is
//! open at a time; a second of that kind is refused as busy, as the team
//! does not order concurrent operations. Every operation ends with one
//! receipt: when its members answered, when an abort ended it, or when an
//! answer is lost.
//!
//! A member request that never went out is refused for that reason. An
//! answer that is lost, through a lost link or a timeout, makes the receipt
//! uncertain: it does not show that the member did not execute the request.
//! So the request stays pending, and an answer that comes after its
//! operation ended still moves the team, but it answers no operator again.

use std::collections::BTreeMap;

use pilotage_protocol::wire;

use super::{MemberIntent, Op, TeamLane};

/// How long an operation waits for its members' answers.
pub(super) const ANSWER_TIMEOUT_MS: u64 = 30_000;

/// One open operation.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Operation {
    kind: Op,
    opened_unix_ms: u64,
    /// Member requests not yet answered, with their members.
    waiting: BTreeMap<String, String>,
    /// Member requests whose answer is lost, with why.
    lost: Vec<String>,
}

fn name(kind: Op) -> &'static str {
    match kind {
        Op::Propose => "proposal",
        Op::Approve => "approval",
        Op::Start => "start",
        Op::Revise => "revision",
        Op::Renew => "renewal",
    }
}

impl TeamLane {
    /// Refuses a second operation of `kind` while one is open.
    pub(super) fn busy(&self, kind: Op) -> Result<(), String> {
        match self.operations.iter().find(|(_, open)| open.kind == kind) {
            Some((id, open)) => Err(format!(
                "busy: the {} {id} still waits for {}",
                name(kind),
                open.waiting
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            None => Ok(()),
        }
    }

    /// Opens the operation the operator's `request_id` asked for, over the
    /// member requests `intents`, at `now_unix_ms`. Returns whether it waits
    /// for any member.
    pub(super) fn open_operation(
        &mut self,
        request_id: &str,
        intents: &[MemberIntent],
        now_unix_ms: u64,
    ) -> bool {
        let mut waiting = BTreeMap::new();
        let mut kind = None;
        for intent in intents {
            if let Some(pending) = self.pending.get_mut(&intent.intent.request_id) {
                pending.operation = Some(request_id.to_owned());
                kind = Some(pending.op);
                waiting.insert(intent.intent.request_id.clone(), intent.member.clone());
            }
        }
        let Some(kind) = kind else {
            return false;
        };
        self.operations.insert(
            request_id.to_owned(),
            Operation {
                kind,
                opened_unix_ms: now_unix_ms,
                waiting,
                lost: Vec::new(),
            },
        );
        true
    }

    /// The member request `child` of `operation` is answered.
    pub(super) fn answered(&mut self, operation: Option<&str>, child: &str) {
        if let Some(open) = operation.and_then(|id| self.operations.get_mut(id)) {
            open.waiting.remove(child);
        }
    }

    /// Ends `operation` uncertain when it waits for nobody but lost an
    /// answer: the members left answered, and the lost answer may never
    /// come.
    pub(super) fn close_if_only_lost(
        &mut self,
        operation: Option<&str>,
    ) -> Option<wire::MissionReceipt> {
        let open = self.operations.get(operation?)?;
        if !open.waiting.is_empty() || open.lost.is_empty() {
            return None;
        }
        self.finish(operation, true, String::new(), None)
    }

    /// Whether a member request of the same operation as `operation`, or of
    /// the team step `op` when it has none, still waits for its answer.
    pub(super) fn group_waits(&self, operation: Option<&str>, op: Op) -> bool {
        self.pending.values().any(|p| match operation {
            Some(id) => p.operation.as_deref() == Some(id),
            None => p.refly.is_none() && !p.revision && p.op == op,
        })
    }

    /// Drops the member requests still pending of `operation`, or of the
    /// team step `op` when it has none: their answers take no step.
    pub(super) fn drop_group(&mut self, operation: Option<&str>, op: Op) {
        self.pending.retain(|_, p| match operation {
            Some(id) => p.operation.as_deref() != Some(id),
            None => p.refly.is_some() || p.revision || p.op != op,
        });
    }

    /// Ends `operation` with its receipt, unless it ended already.
    pub(super) fn finish(
        &mut self,
        operation: Option<&str>,
        accepted: bool,
        reason: String,
        proposal: Option<wire::MissionProposalRecord>,
    ) -> Option<wire::MissionReceipt> {
        let id = operation?;
        let ended = self.operations.remove(id)?;
        if ended.lost.is_empty() {
            return Some(self.receipt(id.to_owned(), accepted, reason, proposal));
        }
        let unknown = format!(
            "not known whether these executed it: {}",
            ended.lost.join("; ")
        );
        let mut receipt = self.receipt(
            id.to_owned(),
            accepted,
            if reason.is_empty() {
                unknown
            } else {
                format!("{reason}; {unknown}")
            },
            proposal,
        );
        if accepted {
            receipt.outcome = wire::MissionReceiptOutcome::Uncertain as i32;
        }
        Some(receipt)
    }

    /// `member`'s link is lost: its answers to open operations are lost.
    pub(super) fn lose_answers_of(&mut self, member: &str) {
        let ended = self.lose_answers(
            |_, to| to == member,
            "the link was lost before it answered; the request may have reached it",
        );
        self.ended.extend(ended);
    }

    /// The host's clock: an operation that waited too long for an answer
    /// ends uncertain. [`TeamLane::take_ended`] gives its receipt.
    pub fn on_host_clock(&mut self, now_unix_ms: u64) {
        let late: Vec<String> = self
            .operations
            .iter()
            .filter(|(_, o)| now_unix_ms.saturating_sub(o.opened_unix_ms) >= ANSWER_TIMEOUT_MS)
            .map(|(id, _)| id.clone())
            .collect();
        let reason = format!("no answer within {} s", ANSWER_TIMEOUT_MS / 1000);
        let ended = self.lose_answers(|id, _| late.iter().any(|l| l == id), &reason);
        self.ended.extend(ended);
    }

    /// Ends every open operation: an abort ended what it waited for.
    pub(super) fn end_operations(&mut self) {
        let ended = self.lose_answers(|_, _| true, "the team aborted before it answered");
        self.ended.extend(ended);
    }

    /// The receipts of operator requests that ended since the last call,
    /// other than as an answer to an input: by an abort, a lost link, a
    /// timeout or a replaced plan. The host sends each to its operator.
    pub fn take_ended(&mut self) -> Vec<wire::MissionReceipt> {
        std::mem::take(&mut self.ended)
    }

    /// Records as lost each waiting answer `lost` names, by operation and
    /// member, and ends each operation left with nothing to wait for.
    pub(super) fn lose_answers(
        &mut self,
        lost: impl Fn(&str, &str) -> bool,
        why: &str,
    ) -> Vec<wire::MissionReceipt> {
        let mut done = Vec::new();
        for (id, open) in &mut self.operations {
            let (gone, kept): (BTreeMap<_, _>, BTreeMap<_, _>) = std::mem::take(&mut open.waiting)
                .into_iter()
                .partition(|(_, member)| lost(id, member));
            open.waiting = kept;
            open.lost
                .extend(gone.values().map(|member| format!("{member}: {why}")));
            if open.waiting.is_empty() && !open.lost.is_empty() {
                done.push(id.clone());
            }
        }
        done.iter()
            .filter_map(|id| self.finish(Some(id), true, String::new(), None))
            .collect()
    }
}

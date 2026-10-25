//! The coordinator role (ADR-0028, ADR-0052): a host with no vehicle of
//! its own whose mission lane is a team lane. An operator issues a team
//! objective to it as to any vehicle's lane; it works only through its
//! members' lanes, as an ordinary client of each member host, so every
//! member host keeps fencing it like any other principal.

mod adapter;
mod config;
mod journal;
mod member_link;
mod outbox;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pilotage_coordination::{MemberIntent, TeamLane, TeamReply};
use pilotage_mission::{IntentContext, Requester, Steering};
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{error, info, warn};

use super::engine_actor::ToEngine;
use super::mission_lane::LaneInbound;

pub(crate) use adapter::CoordinatorAdapter;
pub(crate) use config::CoordinatorConfig;
use member_link::MemberEvent;

/// The status is published at least this often, for late joiners.
const STATUS_REFRESH: Duration = Duration::from_secs(2);

/// Spawns the team lane and one link per member. Returns the lane's task.
pub(crate) async fn spawn(
    engine_tx: &mpsc::Sender<ToEngine>,
    config: CoordinatorConfig,
) -> JoinHandle<()> {
    let (inbound_tx, inbound) = mpsc::channel(32);
    if engine_tx
        .send(ToEngine::MissionLane { sender: inbound_tx })
        .await
        .is_err()
    {
        warn!("the engine actor is gone; the coordinator does not start");
    }
    let (events_tx, events) = mpsc::channel(256);
    let mut links = BTreeMap::new();
    for member in &config.members {
        let (intents_tx, intents) = mpsc::channel(32);
        member_link::spawn(member.clone(), intents, events_tx.clone());
        links.insert(member.name.clone(), intents_tx);
    }
    info!(team = %config.team.name, members = config.members.len(), "coordinator ready");
    let mut lane = TeamLane::new(config.team);
    if !config.restored.is_empty() {
        warn!(
            handovers = config.restored.len(),
            "the coordinator restarted with handovers in its journal; open ones are unreconciled"
        );
        lane.restore_handovers(&config.restored);
    }
    let dispatch = outbox::Dispatch::new(lane.handovers());
    let task = TeamTask {
        lane,
        journal: config.journal,
        dispatch,
        engine: engine_tx.clone(),
        links,
        operator_requests: BTreeMap::new(),
        ids: 0,
        last_publish: None,
        steering: Steering::default(),
        start: Instant::now(),
    };
    tokio::spawn(async move {
        task.run(inbound, events).await;
        tracing::debug!(task = "coordinator", "task exited");
    })
}

struct TeamTask {
    lane: TeamLane,
    /// Where the handovers are written before any intent leaves.
    journal: std::path::PathBuf,
    /// What waits for the journal before it goes out.
    dispatch: outbox::Dispatch,
    engine: mpsc::Sender<ToEngine>,
    links: BTreeMap<String, mpsc::Sender<wire::MissionIntent>>,
    /// The operator who sent each deferred intent, by request.
    operator_requests: BTreeMap<String, ClientKey>,
    ids: u64,
    last_publish: Option<Instant>,
    steering: Steering,
    /// When the task started, for the team's monotonic clock.
    start: Instant,
}

impl TeamTask {
    async fn run(
        mut self,
        mut inbound: mpsc::Receiver<LaneInbound>,
        mut events: mpsc::Receiver<MemberEvent>,
    ) {
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                message = inbound.recv() => match message {
                    Some(LaneInbound::Intent { client, requester, mut intent }) => {
                        self.on_intent(client, &requester, &mut intent).await;
                    }
                    Some(LaneInbound::Frame(_)) => {}
                    None => break,
                },
                event = events.recv() => match event {
                    Some(event) => self.on_member_event(event).await,
                    None => break,
                },
                _ = ticker.tick() => {
                    let host_ms = self.host_ms();
                    self.lane.on_host_clock(host_ms);
                    self.retry().await;
                }
            }
            // An operator request that ended other than by an answer to it
            // still gets its one receipt.
            for receipt in self.lane.take_ended() {
                self.answer_operator(Some(receipt)).await;
            }
            if !self.publish().await {
                break;
            }
        }
    }

    async fn on_intent(
        &mut self,
        client: ClientKey,
        requester: &Requester,
        intent: &mut wire::MissionIntent,
    ) {
        // An operation the intent opens starts at the host's time now, not
        // at the last tick.
        let host_ms = self.host_ms();
        self.lane.on_host_clock(host_ms);
        if let Err(reason) = self.steering.admit(intent, requester) {
            info!(request = %intent.request_id, %reason, "team intent refused");
            let receipt = Steering::refusal(intent, &self.lane.status().lane, reason);
            self.reply_to(Some(client), receipt).await;
            return;
        }
        let stop = matches!(
            intent.action,
            Some(
                wire::mission_intent::Action::Hold(_)
                    | wire::mission_intent::Action::Abort(_)
                    | wire::mission_intent::Action::Fetch(_)
            )
        );
        if self.dispatch.blocks_new_work() && !stop {
            let receipt =
                Steering::refusal(intent, &self.lane.status().lane, self.dispatch.refusal());
            self.reply_to(Some(client), receipt).await;
            return;
        }
        let context = self.context();
        let reply = self.lane.on_intent(intent, &context);
        match &reply.receipt {
            None => {
                self.operator_requests
                    .insert(intent.request_id.clone(), client);
            }
            Some(receipt) if receipt.outcome == wire::MissionReceiptOutcome::Accepted as i32 => {
                self.steering.accepted(intent, requester);
            }
            Some(_) => {}
        }
        self.answer(reply).await;
    }

    async fn on_member_event(&mut self, event: MemberEvent) {
        let context = self.context();
        match event {
            MemberEvent::Receipt { member, receipt } => {
                let reply = self.lane.on_member_receipt(&member, &receipt, &context);
                self.answer(reply).await;
            }
            MemberEvent::Status { status } => {
                let intents = self.lane.on_member_status(&status, &context);
                self.send(intents).await;
            }
            MemberEvent::Lost { member } => {
                warn!(%member, "member link lost; its work stays reserved to it");
                self.lane.on_member_lost(&member);
            }
        }
    }

    async fn answer(&mut self, reply: TeamReply) {
        self.answer_operator(reply.receipt).await;
        self.send(reply.intents).await;
    }

    async fn answer_operator(&mut self, receipt: Option<wire::MissionReceipt>) {
        if let Some(receipt) = receipt {
            let to = self.operator_requests.remove(&receipt.request_id);
            info!(request = %receipt.request_id, outcome = receipt.outcome, reason = %receipt.reason, "team intent answered");
            if to.is_some() || !receipt.request_id.is_empty() {
                self.reply_to(to, receipt).await;
            }
        }
    }

    /// Tells the lane that each withheld intent never reached its member,
    /// so the member did not execute it. Returns what the lane sends next.
    async fn refuse_withheld(&mut self, withheld: Vec<outbox::Withheld>) -> Vec<MemberIntent> {
        let mut next = Vec::new();
        for outbox::Withheld { intent, reason } in withheld {
            warn!(member = %intent.member, request = %intent.intent.request_id, reason, "intent not sent");
            let context = self.context();
            let reply = self.lane.on_member_unsent(
                &intent.member,
                &intent.intent.request_id,
                reason,
                &context,
            );
            self.answer_operator(reply.receipt).await;
            next.extend(reply.intents);
        }
        next
    }

    async fn reply_to(&self, to: Option<ClientKey>, receipt: wire::MissionReceipt) {
        self.engine
            .send(ToEngine::MissionPublish {
                to,
                payload: Box::new(wire::envelope::Payload::MissionReceipt(receipt)),
            })
            .await
            .ok();
    }

    /// Sends what may go out now and writes the journal for the rest: work
    /// goes out only once the journal holds the handovers it acts on, and a
    /// stop goes out at once.
    async fn send(&mut self, mut intents: Vec<MemberIntent>) {
        // A withheld intent's refusal can make the lane send more; each
        // refusal ends a step, so this ends.
        loop {
            let mut step = self.dispatch.offer(intents, self.lane.handovers());
            let mut unsent = self.deliver(step.send).await;
            if let Some(handovers) = step.write {
                unsent.extend(self.write(handovers).await);
            }
            step.withheld.extend(unsent);
            intents = self.refuse_withheld(step.withheld).await;
            if intents.is_empty() {
                break;
            }
        }
    }

    /// Writes the journal again while it fails; the waiting work goes out
    /// once it succeeds.
    async fn retry(&mut self) {
        if let Some(handovers) = self.dispatch.retry(self.lane.handovers()) {
            let unsent = self.write(handovers).await;
            let next = self.refuse_withheld(unsent).await;
            self.send(next).await;
        }
    }

    /// Writes the journal and sends what waited for it. Returns what could
    /// not be sent.
    async fn write(&mut self, handovers: Vec<wire::MissionHandover>) -> Vec<outbox::Withheld> {
        let written = journal::write(&self.journal, &handovers).await;
        if let Err(failure) = &written {
            error!(journal = %self.journal.display(), %failure, "the handover journal cannot be written; work waits for it");
        }
        let released = self.dispatch.written(handovers, written.is_ok());
        self.deliver(released).await
    }

    /// Sends each intent on its member's link. Returns those that did not
    /// leave: the member never saw them.
    async fn deliver(&self, intents: Vec<MemberIntent>) -> Vec<outbox::Withheld> {
        let mut unsent = Vec::new();
        for MemberIntent { member, intent } in intents {
            let reason = match self.links.get(&member) {
                Some(link) => match link.send(intent).await {
                    Ok(()) => continue,
                    Err(mpsc::error::SendError(intent)) => (intent, "the member link is stopped"),
                },
                None => (intent, "there is no link to the member"),
            };
            unsent.push(outbox::Withheld {
                intent: MemberIntent {
                    member,
                    intent: reason.0,
                },
                reason: reason.1,
            });
        }
        unsent
    }

    async fn publish(&mut self) -> bool {
        let stale = self
            .last_publish
            .is_none_or(|at| at.elapsed() >= STATUS_REFRESH);
        let mut status = match self.lane.take_changed() {
            Some(status) => status,
            None if stale => self.lane.status(),
            None => return true,
        };
        if self.dispatch.blocks_new_work() {
            status.detail = format!("{}; {}", self.dispatch.refusal(), status.detail);
        }
        self.last_publish = Some(Instant::now());
        self.engine
            .send(ToEngine::MissionPublish {
                to: None,
                payload: Box::new(wire::envelope::Payload::MissionStatus(status)),
            })
            .await
            .is_ok()
    }

    /// Milliseconds since the task started, on the host's monotonic clock.
    fn host_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn context(&mut self) -> IntentContext {
        self.ids = self.ids.wrapping_add(1);
        let now_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            });
        IntentContext {
            now_unix_ms,
            fresh_id: format!("team-{now_unix_ms}-{}", self.ids),
        }
    }
}

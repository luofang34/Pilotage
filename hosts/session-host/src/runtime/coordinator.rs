//! The coordinator role (ADR-0028, ADR-0052): a host with no vehicle of
//! its own whose mission lane is a team lane. An operator issues a team
//! objective to it as to any vehicle's lane; it works only through its
//! members' lanes, as an ordinary client of each member host, so every
//! member host keeps fencing it like any other principal.

mod adapter;
mod config;
mod journal;
mod member_link;

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
    let journaled = lane.handovers();
    let task = TeamTask {
        lane,
        journal: config.journal,
        journaled,
        engine: engine_tx.clone(),
        links,
        operator_requests: BTreeMap::new(),
        ids: 0,
        last_publish: None,
        steering: Steering::default(),
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
    /// The handovers as last written.
    journaled: Vec<wire::MissionHandover>,
    engine: mpsc::Sender<ToEngine>,
    links: BTreeMap<String, mpsc::Sender<wire::MissionIntent>>,
    /// The operator who sent each deferred intent, by request.
    operator_requests: BTreeMap<String, ClientKey>,
    ids: u64,
    last_publish: Option<Instant>,
    steering: Steering,
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
                _ = ticker.tick() => {}
            }
            self.journal().await;
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
        if let Err(reason) = self.steering.admit(intent, requester) {
            info!(request = %intent.request_id, %reason, "team intent refused");
            let receipt = Steering::refusal(intent, &self.lane.status().lane, reason);
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
        if let Some(receipt) = reply.receipt {
            let to = self.operator_requests.remove(&receipt.request_id);
            info!(request = %receipt.request_id, outcome = receipt.outcome, reason = %receipt.reason, "team intent answered");
            if to.is_some() || !receipt.request_id.is_empty() {
                self.reply_to(to, receipt).await;
            }
        }
        self.send(reply.intents).await;
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

    /// Writes the handovers when they changed. Returns whether the journal
    /// holds them.
    async fn journal(&mut self) -> bool {
        let handovers = self.lane.handovers();
        if handovers == self.journaled {
            return true;
        }
        match journal::write(&self.journal, &handovers).await {
            Ok(()) => {
                self.journaled = handovers;
                true
            }
            Err(failure) => {
                error!(journal = %self.journal.display(), %failure, "the handover journal cannot be written");
                false
            }
        }
    }

    /// Sends `intents` once the journal holds the handovers they act on; an
    /// intent is never sent ahead of the record of the work it gives.
    async fn send(&mut self, intents: Vec<MemberIntent>) {
        if !self.journal().await {
            error!(
                intents = intents.len(),
                "member intents not sent: the handover journal is not written"
            );
            return;
        }
        for MemberIntent { member, intent } in intents {
            match self.links.get(&member) {
                Some(link) => {
                    if link.send(intent).await.is_err() {
                        warn!(%member, "member link stopped; intent not sent");
                    }
                }
                None => warn!(%member, "no link to member"),
            }
        }
    }

    async fn publish(&mut self) -> bool {
        let stale = self
            .last_publish
            .is_none_or(|at| at.elapsed() >= STATUS_REFRESH);
        let status = match self.lane.take_changed() {
            Some(status) => status,
            None if stale => self.lane.status(),
            None => return true,
        };
        self.last_publish = Some(Instant::now());
        self.engine
            .send(ToEngine::MissionPublish {
                to: None,
                payload: Box::new(wire::envelope::Payload::MissionStatus(status)),
            })
            .await
            .is_ok()
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

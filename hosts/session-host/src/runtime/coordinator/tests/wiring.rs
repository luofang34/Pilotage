//! The coordinator task over a real team and real member lanes while its
//! journal cannot be written: new work is refused, withheld work reaches
//! the team as refused, and an abort ends waiting work for good.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_coordination::{Member, MemberIntent, TeamConfig, TeamLane};
use pilotage_mission::{
    ExecutionTarget, IntentContext, Lane, LaneConfig, MissionConfig, OwnshipSource, Requester,
    SurveyDefaults,
};
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use wire::mission_intent::Action;

use super::super::ToEngine;
use super::super::{TeamTask, outbox::BACKLOG_LIMIT, outbox::Dispatch};

fn point(latitude_deg: f64, longitude_deg: f64) -> wire::GeoPoint {
    wire::GeoPoint {
        latitude_deg,
        longitude_deg,
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn defaults() -> SurveyDefaults {
    SurveyDefaults {
        height_m: 60.0,
        cruise_mps: 10.0,
        ..SurveyDefaults::default()
    }
}

fn vehicle_lane(name: &str) -> Lane {
    let mut template = MissionConfig::new(
        String::new(),
        GeodeticPosition::new(0.0, 0.0, 0.0),
        ClockDomainId::new(1),
    );
    template.ownship_source = OwnshipSource::vehicle_estimate();
    template.execution_target = ExecutionTarget::Simulator;
    Lane::new(LaneConfig {
        name: name.to_owned(),
        simulated: true,
        execution_strategy: "host-guided-velocity".to_owned(),
        template,
        defaults: defaults(),
        usable_energy_wh: None,
        cruise_power_w: None,
        recovery_policy: "hold".to_owned(),
        judged: true,
        evidence_window_ms: 60_000,
    })
}

fn objective() -> wire::MissionObjective {
    wire::MissionObjective {
        objective_id: "team-objective".to_owned(),
        revision: 1,
        title: "battlefield".to_owned(),
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area: vec![
                    point(40.3330, -74.6720),
                    point(40.3330, -74.6650),
                    point(40.3366, -74.6650),
                    point(40.3366, -74.6720),
                ],
                launch: Some(point(40.3328, -74.6722)),
                min_usable_frames_per_line: 2,
                return_to_launch: true,
                ..wire::SurveyObjective::default()
            },
        )),
        ..wire::MissionObjective::default()
    }
}

fn approval(digest: &[u8]) -> wire::MissionApproval {
    wire::MissionApproval {
        approval_id: "team-approval".to_owned(),
        proposal_digest: digest.to_vec(),
        envelope: Some(wire::AdaptationEnvelope {
            max_height_m: 120.0,
            expires_unix_ms: now_unix_ms() + 3_600_000,
            permit_execute: true,
            ..wire::AdaptationEnvelope::default()
        }),
        proposal_format: wire::MissionProposalFormat::V1 as i32,
        ..wire::MissionApproval::default()
    }
}

/// A coordinator task whose journal cannot be written, its member lanes,
/// what each member link and the engine received, and the journal's path.
struct Rig {
    task: TeamTask,
    members: BTreeMap<String, Lane>,
    links: BTreeMap<String, mpsc::Receiver<wire::MissionIntent>>,
    engine: mpsc::Receiver<ToEngine>,
    journal: std::path::PathBuf,
    _dir: tempfile::TempDir,
    ids: u64,
}

impl Rig {
    /// A team of SIM-1 and SIM-2 with a proposal its members drafted. The
    /// journal last held a handover the lane does not have, so the next
    /// send writes, and fails.
    fn proposed() -> (Self, Vec<u8>) {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("not-yet").join("TEAM.handovers");
        let members = [
            ("SIM-1", point(40.3328, -74.6722)),
            ("SIM-2", point(40.3368, -74.6648)),
        ];
        let lane = TeamLane::new(TeamConfig {
            name: "TEAM".to_owned(),
            members: members
                .iter()
                .map(|(name, launch)| Member {
                    name: (*name).to_owned(),
                    launch: *launch,
                    launch_altitude_m: 0.0,
                })
                .collect(),
            defaults: defaults(),
            recovery_policy: "hold".to_owned(),
        });
        let (engine_tx, engine) = mpsc::channel(64);
        let mut links = BTreeMap::new();
        let mut senders = BTreeMap::new();
        for (name, _) in members {
            let (tx, rx) = mpsc::channel(64);
            senders.insert(name.to_owned(), tx);
            links.insert(name.to_owned(), rx);
        }
        let stale = wire::MissionHandover {
            assignment_id: "TEAM/run/handover-1".to_owned(),
            ..wire::MissionHandover::default()
        };
        let task = TeamTask {
            lane,
            journal: journal.clone(),
            dispatch: Dispatch::new(vec![stale]),
            engine: engine_tx,
            links: senders,
            operator_requests: BTreeMap::new(),
            ids: 0,
            last_publish: None,
            steering: pilotage_mission::Steering::default(),
        };
        let mut rig = Self {
            task,
            members: members
                .iter()
                .map(|(name, _)| ((*name).to_owned(), vehicle_lane(name)))
                .collect(),
            links,
            engine,
            journal,
            _dir: dir,
            ids: 0,
        };
        let context = rig.context();
        let reply = rig.task.lane.on_intent(
            &intent("op-propose", Action::Propose(objective())),
            &context,
        );
        let decided = rig.route(reply.intents);
        let digest = decided
            .first()
            .and_then(|r| r.proposal_record.clone())
            .map(|record| record.digest)
            .expect("a team proposal");
        (rig, digest)
    }

    fn context(&mut self) -> IntentContext {
        self.ids = self.ids.wrapping_add(1);
        IntentContext {
            now_unix_ms: now_unix_ms(),
            fresh_id: format!("rig-{}", self.ids),
        }
    }

    /// Delivers member intents straight to the member lanes, past the
    /// outbox, until the exchange settles: the setup the tests start from.
    fn route(&mut self, mut intents: Vec<MemberIntent>) -> Vec<wire::MissionReceipt> {
        let mut decided = Vec::new();
        while let Some(MemberIntent { member, intent }) = intents.pop() {
            let ctx = self.context();
            let Some(lane) = self.members.get_mut(&member) else {
                continue;
            };
            let reply = lane.on_intent(&intent, &ctx);
            let team = self
                .task
                .lane
                .on_member_receipt(&member, &reply.receipt, &ctx);
            decided.extend(team.receipt);
            intents.extend(team.intents);
            if let Some(status) = lane.take_changed() {
                intents.extend(self.task.lane.on_member_status(&status, &ctx));
            }
        }
        decided
    }

    /// An operator intent through the coordinator task, as a client that
    /// holds the approver secret sends it.
    async fn operator(&mut self, request_id: &str, action: Action) {
        let requester = Requester {
            subject: "operator".to_owned(),
            may_approve: true,
        };
        let mut intent = intent(request_id, action);
        self.task
            .on_intent(ClientKey::new(1), &requester, &mut intent)
            .await;
    }

    /// The operator receipt the coordinator published for `request_id`.
    fn receipt(&mut self, request_id: &str) -> Option<wire::MissionReceipt> {
        std::iter::from_fn(|| self.engine.try_recv().ok()).find_map(|message| match message {
            ToEngine::MissionPublish { payload, .. } => match *payload {
                wire::envelope::Payload::MissionReceipt(r) if r.request_id == request_id => Some(r),
                _ => None,
            },
            _ => None,
        })
    }

    /// What each member link received, in order.
    fn sent(&mut self) -> Vec<(String, &'static str)> {
        let mut sent = Vec::new();
        for (member, link) in &mut self.links {
            while let Ok(intent) = link.try_recv() {
                let kind = match intent.action {
                    Some(Action::Start(_)) => "start",
                    Some(Action::Abort(_)) => "abort",
                    Some(Action::Approve(_)) => "approve",
                    _ => "other",
                };
                sent.push((member.clone(), kind));
            }
        }
        sent
    }
}

fn intent(request_id: &str, action: Action) -> wire::MissionIntent {
    wire::MissionIntent {
        request_id: request_id.to_owned(),
        action: Some(action),
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

fn refused(receipt: &wire::MissionReceipt) -> bool {
    receipt.outcome == wire::MissionReceiptOutcome::Refused as i32
}

#[test]
fn new_work_is_refused_while_the_journal_fails() {
    runtime().block_on(async {
        let (mut rig, digest) = Rig::proposed();
        rig.task.send(Vec::new()).await;
        assert!(rig.task.dispatch.blocks_new_work());
        rig.operator("op-approve", Action::Approve(approval(&digest)))
            .await;
        let receipt = rig.receipt("op-approve").expect("answered at once");
        assert!(refused(&receipt), "{}", receipt.reason);
        assert!(
            receipt.reason.contains("cannot write its handover journal"),
            "{}",
            receipt.reason
        );
        assert!(rig.sent().is_empty(), "no member hears of it");
    });
}

#[test]
fn work_the_backlog_cannot_hold_reaches_the_team_as_refused() {
    runtime().block_on(async {
        let (mut rig, digest) = Rig::proposed();
        let filler: Vec<MemberIntent> = (1..BACKLOG_LIMIT)
            .map(|n| MemberIntent {
                member: "SIM-9".to_owned(),
                intent: intent(
                    &format!("filler-{n}"),
                    Action::Start(wire::MissionStart::default()),
                ),
            })
            .collect();
        let handovers = rig.task.lane.handovers();
        rig.task.dispatch.offer(filler, handovers);
        rig.operator("op-approve", Action::Approve(approval(&digest)))
            .await;
        let receipt = rig.receipt("op-approve").expect("the operator hears");
        assert!(refused(&receipt), "{}", receipt.reason);
        assert!(
            receipt.reason.contains("backlog is full"),
            "{}",
            receipt.reason
        );
        assert_eq!(
            rig.task.lane.status().state,
            wire::MissionLaneState::Proposed as i32,
            "the team waits for an approval again"
        );
        assert!(rig.sent().is_empty());
    });
}

#[test]
fn an_abort_while_the_journal_fails_ends_the_waiting_starts_for_good() {
    runtime().block_on(async {
        let (mut rig, digest) = Rig::proposed();
        let context = rig.context();
        let reply = rig.task.lane.on_intent(
            &intent("op-approve", Action::Approve(approval(&digest))),
            &context,
        );
        rig.route(reply.intents);
        rig.operator(
            "op-start",
            Action::Start(wire::MissionStart {
                proposal_digest: digest,
            }),
        )
        .await;
        assert!(
            rig.task.dispatch.blocks_new_work(),
            "the starts wait for the journal"
        );
        assert!(rig.sent().is_empty());
        rig.operator("op-abort", Action::Abort(wire::MissionAbort {}))
            .await;
        let receipt = rig.receipt("op-abort").expect("answered");
        assert!(!refused(&receipt), "{}", receipt.reason);
        std::fs::create_dir_all(rig.journal.parent().unwrap()).unwrap();
        rig.task.retry().await;
        assert!(!rig.task.dispatch.blocks_new_work());
        assert_eq!(
            rig.sent(),
            [("SIM-1".to_owned(), "abort"), ("SIM-2".to_owned(), "abort")],
            "no start goes out after the abort"
        );
    });
}

#[test]
fn a_request_a_gone_link_never_took_is_refused_and_the_operator_entry_cleared() {
    runtime().block_on(async {
        let (mut rig, digest) = Rig::proposed();
        std::fs::create_dir_all(rig.journal.parent().unwrap()).unwrap();
        // SIM-2's link task is gone: nothing sent to it leaves.
        rig.links.remove("SIM-2");
        rig.operator("op-approve", Action::Approve(approval(&digest)))
            .await;
        let receipt = rig.receipt("op-approve").expect("the operator hears");
        assert!(refused(&receipt), "{}", receipt.reason);
        assert!(
            receipt.reason.contains("not sent to SIM-2"),
            "{}",
            receipt.reason
        );
        assert!(
            rig.task.operator_requests.is_empty(),
            "the ended request leaves the host's map"
        );
    });
}
